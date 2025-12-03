use crate::config::{AppConfig, Auth, AuthConfig, RouteConfig};
use crate::metrics::Metrics;
use prometheus::{CounterVec, HistogramVec};
use reqwest::Client;
use rest_latency::keycloak_client::KeycloakClient;
use std::collections::HashMap;
use std::time::Instant;
use tokio::sync::watch;
use tokio::time::{Duration, interval};
use tokio_util::sync::CancellationToken;

pub fn spawn_scraper(mut config_rx: watch::Receiver<AppConfig>, metrics: &Metrics, client: Client) {
    let latency = metrics.latency.clone();
    let auth_attempts = metrics.auth_attempts.clone();

    tokio::spawn(async move {
        loop {
            let config = config_rx.borrow_and_update().clone();
            tracing::info!(
                routes = config.routes.len(),
                interval = config.scrape_interval_seconds,
                "Starting scraper"
            );

            let cancel = CancellationToken::new();
            let scraper_handle = spawn_scraper_loop(
                config,
                client.clone(),
                latency.clone(),
                auth_attempts.clone(),
                cancel.clone(),
            );

            // Wait for config change
            if config_rx.changed().await.is_err() {
                tracing::info!("Config channel closed, stopping scraper");
                cancel.cancel();
                break;
            }

            // Config changed - cancel current scraper and restart
            tracing::info!("Config changed, restarting scraper");
            cancel.cancel();
            let _ = scraper_handle.await;
        }
    });
}

fn spawn_scraper_loop(
    config: AppConfig,
    client: Client,
    latency: HistogramVec,
    auth_attempts: CounterVec,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(config.scrape_interval_seconds));
        let mut auth_clients: HashMap<String, KeycloakClient> = HashMap::new();

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::debug!("Scraper cancelled");
                    break;
                }
                _ = ticker.tick() => {
                    for route in &config.routes {
                        check_route(
                            route.clone(),
                            &config.auths,
                            &client,
                            &mut auth_clients,
                            &latency,
                            &auth_attempts,
                        ).await;
                    }
                }
            }
        }
    })
}

async fn check_route(
    route: RouteConfig,
    auths: &[Auth],
    client: &Client,
    auth_clients: &mut HashMap<String, KeycloakClient>,
    latency: &HistogramVec,
    auth_attempts: &CounterVec,
) {
    let mut req = client.get(&route.url);

    if let Some(ref auth_name) = route.auth
        && let Some(auth) = auths.iter().find(|a| &a.name == auth_name)
    {
        match apply_auth(&mut req, auth, auth_clients, auth_attempts, auth_name).await {
            Ok(r) => req = r,
            Err(()) => return,
        }
    }

    let timer = Instant::now();
    match req.send().await {
        Ok(r) => {
            if !r.status().is_success() {
                tracing::warn!(route = %route.name, status = %r.status(), "Non-success response");
            }
            latency
                .with_label_values(&[&route.name])
                .observe(timer.elapsed().as_secs_f64());
        }
        Err(e) => {
            tracing::error!(route = %route.name, error = %e, "Request failed");
        }
    }
}

fn oauth_cache_key(url: &str, realm: &str, client_id: &str, user: &str, pass: &str) -> String {
    format!("{}:{}:{}:{}:{}", url, realm, client_id, user, pass)
}

async fn apply_auth(
    req: &mut reqwest::RequestBuilder,
    auth: &Auth,
    auth_clients: &mut HashMap<String, KeycloakClient>,
    auth_attempts: &CounterVec,
    auth_name: &str,
) -> Result<reqwest::RequestBuilder, ()> {
    let taken = std::mem::replace(req, reqwest::Client::new().get(""));

    let result = match &auth.config {
        AuthConfig::Bearer { token } => taken.bearer_auth(token),
        AuthConfig::Basic { user, pass } => taken.basic_auth(user, Some(pass)),
        AuthConfig::OAuth {
            client_id,
            realm,
            url,
            user,
            pass,
        } => {
            let cache_key = oauth_cache_key(url, realm, client_id, user, pass);
            let keycloak = auth_clients
                .entry(cache_key)
                .or_insert_with(|| KeycloakClient::new(url));

            match keycloak.get_token(realm, client_id, user, pass).await {
                Ok(token) => {
                    auth_attempts.with_label_values(&[auth_name, "success"]).inc();
                    taken.bearer_auth(token)
                }
                Err(e) => {
                    tracing::error!(auth_name = %auth_name, error = %e, "Failed to get OAuth token");
                    auth_attempts.with_label_values(&[auth_name, "failure"]).inc();
                    return Err(());
                }
            }
        }
    };

    Ok(result)
}
