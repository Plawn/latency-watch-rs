# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

rest-latency is a Rust service that monitors HTTP endpoint latencies and exposes them as Prometheus metrics. It periodically fetches configured routes, measures response times, and serves metrics via an HTTP endpoint.

## Build Commands

```bash
# Build all binaries
cargo build

# Build release (optimized, stripped)
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

**Main binary (`server`)**: Located at `src/main.rs`. Runs two concurrent tasks:
1. A background ticker that fetches all configured routes at `scrape_interval_seconds` intervals
2. An Axum HTTP server exposing `/metrics` and `/health` endpoints

**HTTP Endpoints**:
- `/metrics` - Prometheus metrics (route latencies, auth attempts)
- `/health` - Health check (returns 200 OK)

**Graceful shutdown**: Handles SIGINT (Ctrl+C) for clean shutdown.

**Configuration**: Loaded from `config.yaml` with environment variable expansion (`${VAR}` syntax via shellexpand). Supports hot-reload on SIGHUP signal.

**Authentication types** (defined via `AuthConfig` enum):
- `Bearer`: Static token
- `Basic`: Username/password
- `OAuth`: Keycloak-style password grant flow with token caching

**KeycloakClient** (`src/keycloak_client.rs`): Handles OAuth token acquisition with in-memory caching. Tokens are cached until 1 second before expiration.

## Configuration Format

```yaml
listen_addr: "0.0.0.0:9090"
scrape_interval_seconds: 15

auths:
  - name: MyAuth
    config:
      type: OAuth  # or Bearer, Basic
      # OAuth-specific fields: url, realm, client_id, user, pass
      # Bearer: token
      # Basic: user, pass

routes:
  - name: "EndpointName"
    url: https://example.com/api
    auth: MyAuth  # optional, references auth by name
```

## Key Dependencies

- `axum` for HTTP server
- `reqwest` for HTTP client (rustls-tls, no OpenSSL, 30s timeout)
- `prometheus` for metrics collection
- `signal-hook-tokio` for SIGHUP config reload
- `thiserror` for error handling
- `tracing` for structured logging
