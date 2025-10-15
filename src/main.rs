use axum::extract::State;
use axum::{Router, http::StatusCode, response::IntoResponse, routing::get};
use dotenvy::dotenv;
use futures::StreamExt;
use futures::lock::Mutex;
use prometheus::{Encoder, HistogramOpts, HistogramVec, Registry, TextEncoder};
use reqwest::Client;
use rest_latency::keycloak_client::KeycloakClient;
use serde::Deserialize;
use signal_hook::consts::SIGHUP;
use signal_hook_tokio::Signals;
use tracing::Level;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::time::{Duration, interval};
use tracing_subscriber::FmtSubscriber;
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

#[derive(Debug, Deserialize, Clone)]
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

async fn load_config() -> AppConfig {
    // Load and preprocess config with environment variable substitution
    let raw = tokio::fs::read_to_string("config.yaml")
        .await
        .expect("config.yaml not found");
    // Expand ${VAR} or $VAR in the file using current environment
    let expanded = shellexpand::env(&raw)
        .expect("Failed to expand environment variables in config")
        .to_string();
    let cfg: AppConfig = serde_yaml::from_str(&expanded).expect("Invalid config");
    cfg
}

async fn use_config() -> Arc<Mutex<AppConfig>> {
    let cfg = Arc::from(Mutex::new(load_config().await));
    let mut signals = Signals::new([SIGHUP]).expect("failed to register signals");
    let cfg_clone = Arc::clone(&cfg);
    tokio::spawn(async move {
        while let Some(sig) = signals.next().await {
            match sig {
                SIGHUP => {
                    println!("Reloading configuration...");
                    // Lock the mutex and update the config
                    let config = load_config().await;
                    let mut cfg_guard = cfg_clone.lock().await;
                    *cfg_guard = config;
                    println!("Configuration reloaded!");
                }
                _ => unreachable!(),
            }
        }
    });
    cfg
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    // Set up structured logging using the `tracing` crate.
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
    let cfg = use_config().await;

    // Metrics registry
    let registry = Registry::new();
    let histogram = HistogramVec::new(
        HistogramOpts::new("route_latency_seconds", "HTTP route latency in seconds").buckets(vec![
            0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.375, 0.5, 0.75, 1.0, 2.5, 5.0, 10.0,
        ]),
        &["route"],
    )
    .expect("metric creation failed");
    registry.register(Box::new(histogram.clone())).unwrap();

    let addr: SocketAddr = cfg
        .lock()
        .await
        .listen_addr
        .parse()
        .expect("Invalid listen_addr");

    let hist = histogram.clone();
    let interval_secs = cfg.lock().await.scrape_interval_seconds;
    tokio::spawn(async move {
        let client = Client::new();
        let mut ticker = interval(Duration::from_secs(interval_secs));
        let mut warmups_done = HashSet::new();
        let mut auth_clients: HashMap<String, KeycloakClient> = HashMap::new();
        loop {
            ticker.tick().await;
            let cfg_guard = cfg.lock().await;
            // Clone or copy the necessary data while holding the lock
            let routes = cfg_guard.routes.clone(); // Make sure `routes` can be cloned
            let auths = cfg_guard.auths.clone(); // Make sure `auths` can be cloned
            drop(cfg_guard); // Explicitly drop the lock as soon as possible
            for route in &routes {
                let mut req = client.get(&route.url);
                if let Some(auth_name) = &route.auth {
                    let _auth = auths.iter().find(|e| &e.name == auth_name).cloned();
                    if let Some(auth) = _auth {
                        match auth.config {
                            AuthConfig::Bearer { ref token } => {
                                req = req.bearer_auth(token);
                            }
                            AuthConfig::Basic { ref user, ref pass } => {
                                req = req.basic_auth(user, Some(pass));
                            }
                            AuthConfig::OAuth {
                                ref client_id,
                                ref realm,
                                url,
                                ref user,
                                ref pass,
                            } => {
                                let u = url.clone();
                                let auth = auth_clients
                                    .entry(u)
                                    .or_insert_with(|| KeycloakClient::new(&url));
                                let t = auth.get_token(realm, client_id, user, pass).await;
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
                let ok = match resp {
                    Ok(r) => {
                        if !r.status().is_success() {
                            eprintln!("Non 200 response code, got {}", r.status().as_str());
                        }
                        true
                    }
                    Err(e) => {
                        eprintln!("Error fetching {}: {}", route.name, e);
                        false
                    }
                };
                if ok {
                    // could be optimized
                    if warmups_done.contains(&route.name) {
                        // we should drop the first one, as tls negociation is slow
                        let elapsed = timer.elapsed().as_secs_f64();
                        println!("{} -> {}", &route.name, &elapsed);
                        hist.with_label_values(&[&route.name]).observe(elapsed);
                    } else {
                        warmups_done.insert(route.name.clone());
                        println!("drop first query; this is tls warmup");
                    }
                }
            }
        }
    });
    let app_state = Arc::new(AppState { registry });
    // Build Axum app
    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .with_state(app_state);

    // Parse listen address

    println!("Starting metrics server on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
    Ok(())
}

// --- Application State ---
// By creating a dedicated AppState struct, we can use Axum's `State` extractor,
// making our handlers cleaner and state management more explicit.
struct AppState {
    registry: Registry,
}

/// A dedicated, clean handler for the /metrics endpoint.
/// It uses the `State` extractor to gain access to the application state (the registry)
/// in an idiomatic way.
async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let metric_families = state.registry.gather();
    let mut buffer = Vec::new();

    // The .unwrap() is safe here as encoding to a Vec<u8> should not fail.
    encoder.encode(&metric_families, &mut buffer).unwrap();

    (
        StatusCode::OK,
        [("Content-Type", encoder.format_type().to_string())],
        buffer,
    )
}
