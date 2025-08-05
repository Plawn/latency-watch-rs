use axum::{Router, http::StatusCode, response::IntoResponse, routing::get};
use prometheus::{Encoder, HistogramOpts, HistogramVec, Registry, TextEncoder};
use reqwest::Client;
use rest_latency::keycloak_client::KeycloakClient;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::net::SocketAddr;
use std::time::Instant;
use tokio::time::{Duration, interval};
use dotenvy::dotenv;

#[derive(Debug, Deserialize, Clone)]
struct RouteConfig {
    name: String,
    url: String,
    auth: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type")]
enum AuthConfig {
    Bearer {
        token: String,
    },
    OAuth {
        user: String,
        pass: String,
        url: String,
        client_id: String,
        realm: String,
    },
    Basic {
        user: String,
        pass: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct Auth {
    name: String,
    config: AuthConfig,
}

#[derive(Debug, Deserialize)]
struct AppConfig {
    routes: Vec<RouteConfig>,
    auths: Vec<Auth>,
    scrape_interval_seconds: u64,
    listen_addr: String,
}

#[tokio::main]
async fn main() {
    dotenv().ok();

    // Load and preprocess config with environment variable substitution
    let raw = fs::read_to_string("config.yaml").expect("config.yaml not found");
    // Expand ${VAR} or $VAR in the file using current environment
    let expanded = shellexpand::env(&raw)
        .expect("Failed to expand environment variables in config")
        .to_string();
    let cfg: AppConfig = serde_yaml::from_str(&expanded).expect("Invalid config");

    // Metrics registry
    let registry = Registry::new();
    let histogram = HistogramVec::new(
        HistogramOpts::new("route_latency_seconds", "HTTP route latency in seconds"),
        &["route"],
    )
    .expect("metric creation failed");
    registry.register(Box::new(histogram.clone())).unwrap();

    // HTTP client
    let client = Client::new();
    // Spawn scraper task
    let routes = cfg.routes.clone();
    let hist = histogram.clone();
    let interval_secs = cfg.scrape_interval_seconds;
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(interval_secs));
        let mut warmups_done = HashSet::new();
        loop {
            ticker.tick().await;
            for route in &routes {
                let mut req = client.get(&route.url);
                if let Some(auth_name) = &route.auth {
                    let _auth = cfg.auths.iter().find(|e| &e.name == auth_name);
                    if let Some(auth) = _auth {
                        match auth.config {
                            AuthConfig::Bearer { ref token } => {
                                req = req.bearer_auth(&token);
                            }
                            AuthConfig::Basic { ref user, ref pass } => {
                                req = req.basic_auth(user, Some(pass));
                            }
                            AuthConfig::OAuth {
                                ref client_id,
                                ref realm,
                                ref url,
                                ref user,
                                ref pass,
                            } => {
                                let auth = KeycloakClient::new(&url);
                                let t = auth.get_token(&realm, &client_id, &user, &pass).await;
                                if let Ok(token) = t {
                                    req = req.bearer_auth(token);
                                } else {
                                    eprintln!("failed to get token for {}", &auth_name);
                                    continue;
                                }
                            }
                        }
                    }
                }
                let timer = Instant::now();
                let resp = req.send().await;
                match resp {
                    Ok(r) => {
                        if !r.status().is_success() {
                            eprintln!("Non 200 response code, got {}", r.status().as_str());
                        }
                    }
                    Err(e) => {
                        eprintln!("Error fetching {}: {}", route.name, e);
                    }
                };
                // could be optimized
                if warmups_done.contains(&route.name) {
                    // we should drop the first one, as tls negociation is slow
                    let elapsed = timer.elapsed().as_secs_f64();
                    println!("took: {}", &elapsed);
                    hist.with_label_values(&[&route.name]).observe(elapsed);
                } else {
                    warmups_done.insert(route.name.clone());
                    println!("drop first query; this is tls warmup");
                }
            }
        }
    });

    // Build Axum app
    let app = Router::new().route(
        "/metrics",
        get(move || {
            let registry = registry.clone();
            async move {
                let encoder = TextEncoder::new();
                let metric_families = registry.gather();
                let mut buffer = Vec::new();
                encoder.encode(&metric_families, &mut buffer).unwrap();
                (
                    StatusCode::OK,
                    [("Content-Type", encoder.format_type().to_string())],
                    buffer,
                )
                    .into_response()
            }
        }),
    );

    // Parse listen address
    let addr: SocketAddr = cfg.listen_addr.parse().expect("Invalid listen_addr");
    println!("Starting metrics server on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
