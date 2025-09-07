use anyhow::{bail, Result};
use log::info;
use solana_client::rpc_client::RpcClient;
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Wait for validator to be ready (both RPC and WebSocket)
/// Times out after 5 minutes
pub async fn wait_for_validator(rpc_url: &str, ws_url: &str) -> Result<()> {
    info!("Waiting for validator to be ready...");
    
    let start = Instant::now();
    let timeout = Duration::from_secs(300); // 5 minutes
    let mut delay = Duration::from_secs(1);
    let max_delay = Duration::from_secs(30);
    let mut attempt = 0;
    
    while start.elapsed() < timeout {
        attempt += 1;
        info!("  Checking validator availability... (attempt {})", attempt);
        
        // Check if both RPC and WebSocket are accessible
        if check_validator_ready(rpc_url, ws_url).await {
            info!("  ✓ Validator is ready (RPC and WebSocket responding)");
            return Ok(());
        }
        
        // Not ready yet
        info!("  Validator not ready yet, retrying in {:?}...", delay);
        sleep(delay).await;
        
        // Exponential backoff
        delay = (delay * 2).min(max_delay);
    }
    
    bail!("Validator failed to become ready within 5 minutes")
}

/// Check if validator is ready by testing both RPC and WebSocket
async fn check_validator_ready(rpc_url: &str, ws_url: &str) -> bool {
    // Check RPC endpoint
    if !check_rpc_ready(rpc_url).await {
        return false;
    }
    
    // Check WebSocket endpoint with a simple TCP connection
    // (Full WebSocket handshake would require more complexity)
    check_websocket_ready(ws_url).await
}

/// Check if RPC endpoint is ready
async fn check_rpc_ready(rpc_url: &str) -> bool {
    // Use tokio::task::spawn_blocking to safely handle potentially blocking RPC call
    match tokio::task::spawn_blocking({
        let url = rpc_url.to_string();
        move || {
            std::panic::catch_unwind(|| {
                RpcClient::new(url).get_version()
            })
        }
    }).await {
        Ok(Ok(Ok(_))) => true,
        _ => false,
    }
}

/// Check if WebSocket endpoint is ready
async fn check_websocket_ready(ws_url: &str) -> bool {
    // Extract host and port from ws_url
    let url = match url::Url::parse(ws_url) {
        Ok(u) => u,
        Err(_) => return false,
    };
    
    let host = match url.host_str() {
        Some(h) => h,
        None => return false,
    };
    
    let port = url.port().unwrap_or(if url.scheme() == "wss" { 443 } else { 80 });
    
    // Try to connect with TCP
    match tokio::net::TcpStream::connect((host, port)).await {
        Ok(_) => true,
        Err(_) => false,
    }
}