use prometheus::{CounterVec, HistogramOpts, HistogramVec, Opts, Registry};

pub struct Metrics {
    pub registry: Registry,
    pub latency: HistogramVec,
    pub auth_attempts: CounterVec,
}

impl Metrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let latency = HistogramVec::new(
            HistogramOpts::new("route_latency_seconds", "HTTP route latency in seconds")
                .buckets(vec![
                    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.375, 0.5, 0.75, 1.0, 2.5, 5.0, 10.0,
                ]),
            &["route"],
        )?;
        registry.register(Box::new(latency.clone()))?;

        let auth_attempts = CounterVec::new(
            Opts::new("auth_attempts_total", "Authentication attempts by status"),
            &["auth_name", "status"],
        )?;
        registry.register(Box::new(auth_attempts.clone()))?;

        Ok(Self {
            registry,
            latency,
            auth_attempts,
        })
    }
}
