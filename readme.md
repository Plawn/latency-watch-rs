# rest-latency

A Rust service that monitors HTTP endpoint latencies and exposes them as Prometheus metrics.

## Features

- Periodic HTTP endpoint monitoring with configurable scrape intervals
- Prometheus metrics endpoint (`/metrics`)
- Multiple authentication methods: Bearer, Basic, OAuth (Keycloak)
- OAuth token caching with automatic refresh
- Hot-reload configuration via SIGHUP signal
- Environment variable expansion in config (`${VAR}` syntax)
- Graceful shutdown handling

## Installation

```bash
# Build
cargo build --release

# The binary will be at target/release/server
```

## Usage

```bash
# Create a config.yaml (see Configuration section)
# Run the server
cargo run --bin server
```

## Configuration

Create a `config.yaml` file in the working directory:

```yaml
listen_addr: "0.0.0.0:9090"
scrape_interval_seconds: 15

auths:
  - name: my-bearer-auth
    config:
      type: Bearer
      token: "${API_TOKEN}"

  - name: my-basic-auth
    config:
      type: Basic
      user: "${BASIC_USER}"
      pass: "${BASIC_PASS}"

  - name: my-oauth
    config:
      type: OAuth
      url: https://keycloak.example.com
      realm: myrealm
      client_id: myclient
      user: "${OAUTH_USER}"
      pass: "${OAUTH_PASS}"

routes:
  - name: "api-health"
    url: https://api.example.com/health

  - name: "api-users"
    url: https://api.example.com/users
    auth: my-bearer-auth

  - name: "protected-endpoint"
    url: https://secure.example.com/data
    auth: my-oauth
```

Environment variables can be used with `${VAR}` syntax. A `.env` file is automatically loaded if present.

## Endpoints

| Endpoint   | Description                          |
|------------|--------------------------------------|
| `/metrics` | Prometheus metrics                   |
| `/health`  | Health check (returns 200 OK)        |

## Prometheus Metrics

| Metric                  | Type      | Labels              | Description                        |
|-------------------------|-----------|---------------------|------------------------------------|
| `route_latency_seconds` | Histogram | `route`             | HTTP route latency in seconds      |
| `auth_attempts_total`   | Counter   | `auth_name`,`status`| Authentication attempts by status  |

## Hot Reload

Send SIGHUP to reload the configuration without restarting:

```bash
kill -HUP $(pgrep server)
```

## TODO

- Add GitHub build actions
- Make OAuth more flexible
