# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

rest-latency is a Rust service that monitors HTTP endpoint latencies and exposes them as Prometheus metrics. It periodically fetches configured routes, measures response times, and serves metrics via an HTTP endpoint.

## Build Commands

```bash
# Build all binaries
cargo build

# Build release (optimized, stripped with LTO)
cargo build --release

# Run the main metrics server
cargo run --bin server

# Run clippy for linting
cargo clippy

# Format code
cargo fmt

# Run tests
cargo test
```

## Architecture

### Binaries

- **`server`** (`src/main.rs`): Main metrics server
- **`cache`** (`src/cache_helper.rs`): Placeholder utility (not yet implemented)

### Module Structure

- `src/main.rs` - Entry point, HTTP server setup, graceful shutdown
- `src/config.rs` - Configuration loading, SIGHUP hot-reload via `watch::channel`
- `src/metrics.rs` - Prometheus metrics definitions (`Metrics` struct)
- `src/scraper.rs` - Background scraping loop with `CancellationToken` for restart
- `src/lib.rs` - Library exports (`keycloak_client` module)
- `src/keycloak_client.rs` - OAuth token acquisition with in-memory caching

### Main Flow

1. Load config via `with_hot_reload()` which returns a `watch::Receiver<AppConfig>`
2. Start `spawn_scraper()` which listens for config changes and restarts on update
3. Run Axum HTTP server with `/metrics` and `/health` endpoints
4. Handle SIGINT for graceful shutdown

### Scraper Architecture

The scraper uses a `CancellationToken` pattern:
- When config changes, the current scraper loop is cancelled
- A new scraper loop starts with the updated config
- This ensures seamless config hot-reload without server restart

### Prometheus Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `route_latency_seconds` | Histogram | `route` | HTTP route latency (buckets: 5ms to 10s) |
| `auth_attempts_total` | Counter | `auth_name`, `status` | OAuth auth attempts (success/failure) |

### Authentication Types

Defined via `AuthConfig` enum in `src/config.rs`:
- **Bearer**: Static token (`token` field)
- **Basic**: Username/password (`user`, `pass` fields)
- **OAuth**: Keycloak password grant (`url`, `realm`, `client_id`, `user`, `pass`)

**KeycloakClient** caches tokens until 1 second before expiration and auto-cleans expired tokens.

## Configuration Format

```yaml
listen_addr: "0.0.0.0:9090"
scrape_interval_seconds: 15

auths:
  - name: MyAuth
    config:
      type: OAuth  # or Bearer, Basic
      url: https://keycloak.example.com
      realm: myrealm
      client_id: myclient
      user: "${OAUTH_USER}"
      pass: "${OAUTH_PASS}"

routes:
  - name: "EndpointName"
    url: https://example.com/api
    auth: MyAuth  # optional, references auth by name
```

Environment variables are expanded via `${VAR}` syntax (shellexpand). A `.env` file is loaded via dotenvy.

## Key Dependencies

- `axum` - HTTP server
- `reqwest` - HTTP client (rustls-tls, no OpenSSL, 30s timeout)
- `prometheus` - Metrics collection
- `tokio` / `tokio-util` - Async runtime, `CancellationToken`
- `signal-hook-tokio` - SIGHUP config reload
- `thiserror` - Error handling
- `tracing` / `tracing-subscriber` - Structured logging
- `chrono` - Token expiration handling
- `serde` / `serde_yaml` - Config deserialization

## Rust Edition

Uses Rust edition 2024.
