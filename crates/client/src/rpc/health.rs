//! Health Check Service
//!
//! Background task that periodically checks endpoint health.

use std::sync::Arc;

use tokio::sync::watch;

use super::config::HealthCheckConfig;
use super::endpoint::EndpointState;

/// Health checker for RPC endpoints
pub struct HealthChecker {
    /// Endpoints to check
    endpoints: Vec<Arc<EndpointState>>,
    /// HTTP client for health checks
    http_client: reqwest::Client,
    /// Configuration
    config: HealthCheckConfig,
    /// Shutdown signal receiver
    shutdown_rx: watch::Receiver<bool>,
}

impl HealthChecker {
    /// Create a new health checker
    pub fn new(
        endpoints: Vec<Arc<EndpointState>>,
        config: HealthCheckConfig,
        shutdown_rx: watch::Receiver<bool>,
    ) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .expect("Failed to build health check HTTP client");

        Self {
            endpoints,
            http_client,
            config,
            shutdown_rx,
        }
    }

    /// Start the health check loop
    pub async fn run(mut self) {
        log::info!(
            "Starting health checker with {} endpoints, interval {:?}",
            self.endpoints.len(),
            self.config.interval
        );

        let mut interval = tokio::time::interval(self.config.interval);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    self.check_all_endpoints().await;
                }
                _ = self.shutdown_rx.changed() => {
                    if *self.shutdown_rx.borrow() {
                        log::info!("Health checker shutting down");
                        break;
                    }
                }
            }
        }
    }

    /// Check all endpoints
    async fn check_all_endpoints(&self) {
        for endpoint in &self.endpoints {
            self.check_endpoint(endpoint).await;
        }
    }

    /// Check a single endpoint's health
    async fn check_endpoint(&self, endpoint: &EndpointState) {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getHealth"
        });

        let start = std::time::Instant::now();
        let result = self
            .http_client
            .post(endpoint.url())
            .json(&body)
            .send()
            .await;

        match result {
            Ok(response) => {
                if response.status().is_success() {
                    // Parse response to check for RPC-level health
                    match response.text().await {
                        Ok(text) => {
                            if text.contains("\"result\":\"ok\"") || text.contains("\"result\": \"ok\"") {
                                endpoint.record_success(start.elapsed());
                                log::trace!("Health check passed for {}", endpoint.url());
                            } else if text.contains("\"error\"") {
                                // RPC returned an error
                                endpoint.record_failure();
                                log::warn!(
                                    "Health check failed for {} (RPC error): {}",
                                    endpoint.url(),
                                    &text[..text.len().min(200)]
                                );
                            } else {
                                // Unknown response, treat as success
                                endpoint.record_success(start.elapsed());
                            }
                        }
                        Err(e) => {
                            endpoint.record_failure();
                            log::warn!(
                                "Health check failed for {} (body read error): {}",
                                endpoint.url(),
                                e
                            );
                        }
                    }
                } else {
                    endpoint.record_failure();
                    log::warn!(
                        "Health check failed for {} (HTTP {})",
                        endpoint.url(),
                        response.status()
                    );
                }
            }
            Err(e) => {
                endpoint.record_failure();
                log::warn!("Health check failed for {} (request error): {}", endpoint.url(), e);
            }
        }
    }
}

/// Spawn the health checker as a background task
pub fn spawn_health_checker(
    endpoints: Vec<Arc<EndpointState>>,
    config: HealthCheckConfig,
) -> (tokio::task::JoinHandle<()>, watch::Sender<bool>) {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let checker = HealthChecker::new(endpoints, config, shutdown_rx);
    let handle = tokio::spawn(checker.run());

    (handle, shutdown_tx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::config::EndpointConfig;
    use std::time::Duration;

    #[tokio::test]
    async fn test_health_checker_creation() {
        let endpoint = Arc::new(EndpointState::new(EndpointConfig::new(
            "https://api.devnet.solana.com",
        )));

        let config = HealthCheckConfig {
            interval: Duration::from_secs(30),
            timeout: Duration::from_secs(5),
            unhealthy_threshold: 3,
        };

        let (_, shutdown_rx) = watch::channel(false);
        let checker = HealthChecker::new(vec![endpoint], config, shutdown_rx);

        assert_eq!(checker.endpoints.len(), 1);
    }

    #[tokio::test]
    async fn test_spawn_health_checker() {
        let endpoint = Arc::new(EndpointState::new(EndpointConfig::new(
            "https://api.devnet.solana.com",
        )));

        let config = HealthCheckConfig {
            interval: Duration::from_secs(60),  // Long interval so it doesn't run
            timeout: Duration::from_secs(5),
            unhealthy_threshold: 3,
        };

        let (handle, shutdown_tx) = spawn_health_checker(vec![endpoint], config);

        // Shutdown immediately
        shutdown_tx.send(true).unwrap();

        // Should complete
        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .expect("Health checker should shut down")
            .expect("Task should complete without panic");
    }
}
