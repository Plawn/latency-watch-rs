use reqwest::Client;
use std::time::Duration;

fn load_env() {
    dotenvy::dotenv().ok();
}

fn get_env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{} must be set in .env", key))
}

mod keycloak {
    use super::*;
    use rest_latency::keycloak_client::KeycloakClient;

    #[tokio::test]
    async fn test_oauth_token_fetch() {
        load_env();

        let mut client = KeycloakClient::new("https://identity.blumana.app");
        let result = client
            .get_token("blumana", "front", &get_env("BLU_USERNAME"), &get_env("BLU_PASSWORD"))
            .await;

        assert!(result.is_ok(), "Failed to get token: {:?}", result.err());
        let token = result.unwrap();
        assert!(!token.is_empty(), "Token should not be empty");
    }

    #[tokio::test]
    async fn test_oauth_token_caching() {
        load_env();

        let mut client = KeycloakClient::new("https://identity.blumana.app");
        let user = get_env("BLU_USERNAME");
        let pass = get_env("BLU_PASSWORD");

        let token1 = client.get_token("blumana", "front", &user, &pass).await.unwrap();
        let token2 = client.get_token("blumana", "front", &user, &pass).await.unwrap();

        assert_eq!(token1, token2, "Second call should return cached token");
    }

    #[tokio::test]
    async fn test_oauth_invalid_credentials() {
        let mut client = KeycloakClient::new("https://identity.blumana.app");
        let result = client
            .get_token("blumana", "front", "invalid_user", "invalid_pass")
            .await;

        assert!(result.is_err(), "Should fail with invalid credentials");
    }
}

mod api {
    use super::*;

    #[tokio::test]
    async fn test_authenticated_api_call() {
        load_env();

        let mut keycloak = rest_latency::keycloak_client::KeycloakClient::new("https://identity.blumana.app");
        let token = keycloak
            .get_token("blumana", "front", &get_env("BLU_USERNAME"), &get_env("BLU_PASSWORD"))
            .await
            .expect("Failed to get token");

        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap();

        let resp = client
            .get("https://api2.temp2-webservice.blumana.app/user/self")
            .bearer_auth(&token)
            .send()
            .await
            .expect("Request failed");

        assert!(
            resp.status().is_success(),
            "API call failed with status: {}",
            resp.status()
        );
    }
}

mod hot_reload {
    use std::fs;
    use std::process::{Child, Command, Stdio};
    use std::io::{BufRead, BufReader};
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    struct ServerProcess {
        child: Child,
        stdout_reader: BufReader<std::process::ChildStdout>,
    }

    impl ServerProcess {
        fn start() -> Self {
            let mut child = Command::new("cargo")
                .args(["run", "--bin", "server"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("Failed to start server");

            let stdout = child.stdout.take().expect("Failed to get stdout");
            let stdout_reader = BufReader::new(stdout);

            Self { child, stdout_reader }
        }

        fn wait_for_log(&mut self, pattern: &str, timeout_secs: u64) -> bool {
            use std::time::{Duration, Instant};
            let start = Instant::now();
            let timeout = Duration::from_secs(timeout_secs);

            while start.elapsed() < timeout {
                let mut line = String::new();
                if self.stdout_reader.read_line(&mut line).unwrap_or(0) > 0 {
                    println!("SERVER: {}", line.trim());
                    if line.contains(pattern) {
                        return true;
                    }
                }
            }
            false
        }

        fn send_sighup(&self) {
            let pid = Pid::from_raw(self.child.id() as i32);
            kill(pid, Signal::SIGHUP).expect("Failed to send SIGHUP");
        }
    }

    impl Drop for ServerProcess {
        fn drop(&mut self) {
            let _ = self.child.kill();
        }
    }

    #[test]
    #[ignore] // Run with: cargo test --test e2e hot_reload -- --ignored
    fn test_config_hot_reload_restarts_scraper() {
        // Backup original config
        let original = fs::read_to_string("config.yaml").expect("Failed to read config");

        // Start server
        let mut server = ServerProcess::start();

        // Wait for initial startup
        assert!(
            server.wait_for_log("Starting scraper", 10),
            "Server didn't start scraper"
        );

        // Modify config (change scrape interval)
        let modified = original.replace("scrape_interval_seconds: 15", "scrape_interval_seconds: 5");
        fs::write("config.yaml", &modified).expect("Failed to write config");

        // Send SIGHUP
        server.send_sighup();

        // Should see reload messages
        assert!(
            server.wait_for_log("Reloading configuration", 5),
            "Didn't see reload message"
        );
        assert!(
            server.wait_for_log("Configuration reloaded successfully", 5),
            "Config reload failed"
        );
        assert!(
            server.wait_for_log("Config changed, restarting scraper", 5),
            "Scraper didn't restart"
        );
        assert!(
            server.wait_for_log("Starting scraper", 5),
            "New scraper didn't start"
        );

        // Restore original config
        fs::write("config.yaml", &original).expect("Failed to restore config");
    }

    #[test]
    #[ignore]
    fn test_invalid_config_reload_keeps_existing() {
        let original = fs::read_to_string("config.yaml").expect("Failed to read config");

        let mut server = ServerProcess::start();
        assert!(server.wait_for_log("Starting scraper", 10), "Server didn't start");

        // Write invalid YAML
        fs::write("config.yaml", "invalid: yaml: content: [[[").expect("Failed to write");

        server.send_sighup();

        // Should see error but keep running
        assert!(
            server.wait_for_log("Failed to reload config", 5),
            "Should log reload failure"
        );

        // Restore and verify we can still reload
        fs::write("config.yaml", &original).expect("Failed to restore");
        server.send_sighup();

        assert!(
            server.wait_for_log("Configuration reloaded successfully", 5),
            "Should reload after fix"
        );

        fs::write("config.yaml", &original).expect("Failed to restore config");
    }
}

mod server {
    use super::*;
    use axum::{Router, routing::get, http::StatusCode};
    use prometheus::{Registry, Encoder, TextEncoder, HistogramVec, HistogramOpts};
    use std::sync::Arc;
    use std::net::SocketAddr;

    async fn spawn_test_server() -> SocketAddr {
        let registry = Registry::new();
        let histogram = HistogramVec::new(
            HistogramOpts::new("route_latency_seconds", "test"),
            &["route"],
        ).unwrap();
        registry.register(Box::new(histogram.clone())).unwrap();
        // Record a sample so the metric appears in output
        histogram.with_label_values(&["test_route"]).observe(0.1);

        let app = Router::new()
            .route("/health", get(|| async { StatusCode::OK }))
            .route("/metrics", get({
                let reg = Arc::new(registry);
                move || {
                    let reg = reg.clone();
                    async move {
                        let encoder = TextEncoder::new();
                        let mut buf = Vec::new();
                        encoder.encode(&reg.gather(), &mut buf).unwrap();
                        (StatusCode::OK, buf)
                    }
                }
            }));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Give server time to start
        tokio::time::sleep(Duration::from_millis(100)).await;
        addr
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let addr = spawn_test_server().await;

        let client = Client::new();
        let resp = client
            .get(format!("http://{}/health", addr))
            .send()
            .await
            .expect("Health check failed");

        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn test_metrics_endpoint() {
        let addr = spawn_test_server().await;

        let client = Client::new();
        let resp = client
            .get(format!("http://{}/metrics", addr))
            .send()
            .await
            .expect("Metrics request failed");

        assert_eq!(resp.status(), 200);

        let body = resp.text().await.unwrap();
        assert!(body.contains("route_latency_seconds"), "Should contain latency metric");
    }
}
