/// Metrics initialization and configuration for the Antegen plugin
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    pub enabled: bool,
    #[serde(default = "default_backend")]
    pub backend: MetricsBackend,
    pub prometheus: Option<PrometheusConfig>,
    pub otlp: Option<OtlpConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricsBackend {
    Prometheus,
    Otlp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrometheusConfig {
    #[serde(default = "default_prometheus_port")]
    pub port: u16,
    #[serde(default = "default_prometheus_path")]
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtlpConfig {
    pub endpoint: String,
    #[serde(default = "default_otlp_protocol")]
    pub protocol: String,
}

fn default_backend() -> MetricsBackend {
    MetricsBackend::Prometheus
}

fn default_prometheus_port() -> u16 {
    9090
}

fn default_prometheus_path() -> String {
    "/metrics".to_string()
}

fn default_otlp_protocol() -> String {
    "grpc".to_string()
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: default_backend(),
            prometheus: Some(PrometheusConfig {
                port: default_prometheus_port(),
                path: default_prometheus_path(),
            }),
            otlp: None,
        }
    }
}

/// Initialize a basic meter provider for metrics collection (always called)
/// Returns the Prometheus registry that contains the metrics
pub fn init_basic_meter_provider() -> Result<prometheus::Registry> {
    use opentelemetry::global;
    use opentelemetry_sdk::metrics::MeterProvider;
    use prometheus::Registry;
    
    // Create a basic meter provider even if we're not exposing metrics
    // This ensures metrics are collected internally
    let registry = Registry::new();
    let exporter = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .build()?;
    
    let provider = MeterProvider::builder()
        .with_reader(exporter)
        .build();
    
    global::set_meter_provider(provider);
    log::debug!("METRICS: Basic meter provider initialized");
    Ok(registry)
}

/// Initialize metrics HTTP server based on configuration
pub async fn init_metrics(config: &MetricsConfig, registry: prometheus::Registry, runtime_handle: tokio::runtime::Handle) -> Result<()> {
    if !config.enabled {
        log::info!("METRICS: Skipping HTTP server (disabled)");
        return Ok(());
    }

    match config.backend {
        MetricsBackend::Prometheus => {
            init_prometheus_metrics(config.prometheus.as_ref()
                .ok_or_else(|| anyhow::anyhow!("Prometheus config required"))?, registry, runtime_handle).await
        }
        MetricsBackend::Otlp => {
            // OTLP support can be added later if needed
            log::warn!("OTLP metrics backend not yet implemented");
            Ok(())
        }
    }
}

async fn init_prometheus_metrics(config: &PrometheusConfig, registry: prometheus::Registry, runtime_handle: tokio::runtime::Handle) -> Result<()> {
    // Use the registry that already has the metrics registered
    // Start HTTP server for Prometheus scraping using the runtime handle
    log::info!("METRICS: Starting Prometheus HTTP server on port {}", config.port);
    runtime_handle.spawn(serve_prometheus_metrics(config.port, config.path.clone(), registry));
    
    log::info!("METRICS: Prometheus endpoint available at http://0.0.0.0:{}{}", config.port, config.path);
    Ok(())
}

async fn serve_prometheus_metrics(port: u16, path: String, registry: prometheus::Registry) {
    use hyper::{
        service::{make_service_fn, service_fn},
        Body, Request, Response, Server, StatusCode,
    };
    use prometheus::{Encoder, TextEncoder};
    
    let make_svc = make_service_fn(move |_conn| {
        let registry = registry.clone();
        let path = path.clone();
        
        async move {
            Ok::<_, hyper::Error>(service_fn(move |req: Request<Body>| {
                let registry = registry.clone();
                let path = path.clone();
                
                async move {
                    if req.uri().path() == path {
                        let encoder = TextEncoder::new();
                        let metric_families = registry.gather();
                        let mut buffer = Vec::new();
                        encoder.encode(&metric_families, &mut buffer).unwrap();
                        
                        Ok::<_, hyper::Error>(Response::new(Body::from(buffer)))
                    } else {
                        Ok(Response::builder()
                            .status(StatusCode::NOT_FOUND)
                            .body(Body::from("Not Found"))
                            .unwrap())
                    }
                }
            }))
        }
    });
    
    let addr = ([0, 0, 0, 0], port).into();
    
    match Server::try_bind(&addr) {
        Ok(server) => {
            log::debug!("METRICS: HTTP server successfully bound to port {}", port);
            if let Err(e) = server.serve(make_svc).await {
                log::error!("METRICS: HTTP server error: {}", e);
            }
        }
        Err(e) => {
            log::error!("METRICS: Failed to bind HTTP server on port {}: {}", port, e);
        }
    }
}


/// Shutdown metrics gracefully
pub fn shutdown() {
    use opentelemetry::global;
    // Note: shutdown_tracer_provider returns () not Result
    global::shutdown_tracer_provider();
    log::info!("METRICS: Provider shut down successfully");
}