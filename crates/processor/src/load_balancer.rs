use anyhow::Result;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration for the load balancer
#[derive(Clone, Debug)]
pub struct LoadBalancerConfig {
    /// Consecutive losses before entering "at capacity" mode
    pub capacity_threshold: u32,
    /// Time to wait before attempting takeover of overdue threads (seconds)
    pub takeover_delay_seconds: i64,
    /// Whether load balancing is enabled
    pub enabled: bool,
}

impl Default for LoadBalancerConfig {
    fn default() -> Self {
        Self {
            capacity_threshold: 5,  // Enter "at capacity" after 5 consecutive losses
            takeover_delay_seconds: 10,  // Wait 10 seconds before takeover attempts
            enabled: true,  // Load balancing enabled by default
        }
    }
}

/// Tracks execution attempts and ownership for threads
#[derive(Clone, Debug, Default)]
pub struct ThreadTracking {
    /// Number of consecutive times we lost execution to another processor
    pub consecutive_losses: u32,
    /// Whether we consider this thread as "owned" by us
    pub owned: bool,
    /// Last time we attempted execution (for rate limiting)
    pub last_attempt: Option<i64>,
}

/// Decision on whether to process a thread
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessDecision {
    /// Process this thread - we own it or it needs takeover
    Process,
    /// Skip this thread - someone else owns it and is keeping it current
    Skip,
    /// We're at capacity, only process if it's critically overdue
    AtCapacity,
}

/// Load balancer that manages thread ownership through natural competition
pub struct LoadBalancer {
    /// Our executor's public key
    pub executor_pubkey: Pubkey,
    /// Configuration
    pub config: LoadBalancerConfig,
    /// Tracking data for each thread
    tracking: Arc<RwLock<HashMap<Pubkey, ThreadTracking>>>,
    /// Whether we're currently at capacity
    at_capacity: Arc<RwLock<bool>>,
}

impl LoadBalancer {
    /// Create a new load balancer
    pub fn new(executor_pubkey: Pubkey, config: LoadBalancerConfig) -> Self {
        Self {
            executor_pubkey,
            config,
            tracking: Arc::new(RwLock::new(HashMap::new())),
            at_capacity: Arc::new(RwLock::new(false)),
        }
    }

    /// Decide whether to process a thread based on ownership and competition
    pub async fn should_process(
        &self,
        thread_pubkey: &Pubkey,
        last_executor: &Pubkey,
        is_overdue: bool,
        overdue_seconds: i64,
    ) -> Result<ProcessDecision> {
        // If load balancing is disabled, always process
        if !self.config.enabled {
            return Ok(ProcessDecision::Process);
        }

        let mut tracking = self.tracking.write().await;
        let thread_track = tracking.entry(*thread_pubkey).or_default();
        
        // Check if we're the last executor
        let we_executed_last = last_executor.eq(&self.executor_pubkey);
        
        // Update ownership based on last executor
        if we_executed_last {
            // We successfully executed - we own this thread
            thread_track.owned = true;
            thread_track.consecutive_losses = 0;
        } else if last_executor.ne(&Pubkey::default()) {
            // Someone else executed - check if we should update ownership
            if thread_track.owned {
                // We thought we owned it but someone else executed
                thread_track.consecutive_losses += 1;
                
                // Check if we should release ownership
                if thread_track.consecutive_losses >= self.config.capacity_threshold {
                    thread_track.owned = false;
                    thread_track.consecutive_losses = 0;
                }
            }
        }
        
        // Check capacity after updating ownership
        let should_check_capacity = thread_track.consecutive_losses >= self.config.capacity_threshold;
        
        // Release the write lock before checking capacity
        drop(tracking);
        
        if should_check_capacity {
            let tracking = self.tracking.read().await;
            self.check_capacity(&tracking).await;
        }
        
        // Re-acquire read access for decision making
        let tracking = self.tracking.read().await;
        let thread_track = tracking.get(thread_pubkey);
        let at_capacity = *self.at_capacity.read().await;
        
        if thread_track.map_or(false, |t| t.owned) {
            // We own this thread - always try to process
            Ok(ProcessDecision::Process)
        } else if is_overdue && overdue_seconds > self.config.takeover_delay_seconds {
            // Thread is overdue beyond takeover delay - attempt takeover
            Ok(ProcessDecision::Process)
        } else if at_capacity {
            // We're at capacity - only process critically overdue threads (1.5x takeover delay)
            if is_overdue && overdue_seconds > (self.config.takeover_delay_seconds * 3) / 2 {
                Ok(ProcessDecision::Process)
            } else {
                Ok(ProcessDecision::AtCapacity)
            }
        } else if last_executor.eq(&Pubkey::default()) {
            // No one has executed this thread yet - try to claim it
            Ok(ProcessDecision::Process)
        } else {
            // Someone else owns this thread and it's current
            Ok(ProcessDecision::Skip)
        }
    }

    /// Record the result of an execution attempt
    pub async fn record_execution_result(
        &self,
        thread_pubkey: &Pubkey,
        success: bool,
        current_timestamp: i64,
    ) -> Result<()> {
        let mut tracking = self.tracking.write().await;
        let thread_track = tracking.entry(*thread_pubkey).or_default();
        
        thread_track.last_attempt = Some(current_timestamp);
        
        if success {
            // We successfully executed
            thread_track.owned = true;
            thread_track.consecutive_losses = 0;
            
            // Reset at_capacity if we successfully took ownership of a new thread
            if !thread_track.owned {
                *self.at_capacity.write().await = false;
            }
        } else {
            // Someone else beat us to execution
            if thread_track.owned {
                thread_track.consecutive_losses += 1;
                
                if thread_track.consecutive_losses >= self.config.capacity_threshold {
                    thread_track.owned = false;
                    thread_track.consecutive_losses = 0;
                    self.check_capacity(&tracking).await;
                }
            }
        }
        
        Ok(())
    }

    /// Check if we should enter "at capacity" mode based on ownership patterns
    async fn check_capacity(&self, tracking: &HashMap<Pubkey, ThreadTracking>) {
        // Count owned threads
        let owned_count = tracking.values().filter(|t| t.owned).count();
        
        // If we have a reasonable number of owned threads and are losing others,
        // we're likely at capacity
        if owned_count > 0 {
            let recent_losses = tracking
                .values()
                .filter(|t| t.consecutive_losses > 0)
                .count();
            
            // If we're losing more threads than we own, we're at capacity
            if recent_losses > owned_count / 2 {
                *self.at_capacity.write().await = true;
            }
        }
    }

    /// Reset tracking for a thread (e.g., after it's deleted)
    pub async fn reset_thread(&self, thread_pubkey: &Pubkey) -> Result<()> {
        let mut tracking = self.tracking.write().await;
        tracking.remove(thread_pubkey);
        Ok(())
    }

    /// Get current statistics for monitoring
    pub async fn get_stats(&self) -> LoadBalancerStats {
        let tracking = self.tracking.read().await;
        let at_capacity = *self.at_capacity.read().await;
        
        LoadBalancerStats {
            total_tracked: tracking.len(),
            owned_threads: tracking.values().filter(|t| t.owned).count(),
            threads_with_losses: tracking.values().filter(|t| t.consecutive_losses > 0).count(),
            at_capacity,
        }
    }
}

/// Statistics for monitoring load balancer performance
#[derive(Debug, Clone)]
pub struct LoadBalancerStats {
    pub total_tracked: usize,
    pub owned_threads: usize,
    pub threads_with_losses: usize,
    pub at_capacity: bool,
}

/// Trait for analyzing thread execution timing
pub trait ThreadExecutionAnalyzer {
    /// Check if a thread is overdue for execution
    fn is_overdue(&self, current_timestamp: i64) -> (bool, i64);
    
    /// Check if the last executor is likely down
    fn is_executor_likely_down(&self, current_timestamp: i64, threshold_seconds: i64) -> bool;
}