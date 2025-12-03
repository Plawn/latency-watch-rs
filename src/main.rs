mod config;
mod metrics;
mod scraper;

use axum::{Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use dotenvy::dotenv;
use prometheus::{Encoder, TextEncoder};
use reqwest::Client;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::time::Duration;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

use crate::config::with_hot_reload;
use crate::metrics::Metrics;
use crate::scraper::spawn_scraper;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .init();

    let config_rx = with_hot_reload().await?;
    let addr: SocketAddr = config_rx.borrow().listen_addr.parse()?;

    let metrics = Metrics::new()?;
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    spawn_scraper(config_rx, &metrics, client);

    let app = Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/health", get(|| async { StatusCode::OK }))
        .with_state(Arc::new(metrics.registry));

    tracing::info!("Starting metrics server on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("Shutting down...");
        })
        .await?;

    Ok(())
}

async fn metrics_handler(State(registry): State<Arc<prometheus::Registry>>) -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();

    if let Err(e) = encoder.encode(&registry.gather(), &mut buffer) {
        tracing::error!(error = %e, "Failed to encode metrics");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to encode metrics").into_response();
    }

    (StatusCode::OK, [("Content-Type", encoder.format_type())], buffer).into_response()
}
