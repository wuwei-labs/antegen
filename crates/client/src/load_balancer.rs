//! Load Balancer for Natural Competition
//!
//! Implements a natural competition model for multi-executor environments.
//! Threads are "owned" when an executor successfully executes them.
//! Ownership is released after consecutive losses to other executors.
//! This prevents duplicate work while allowing takeover of abandoned threads.

use anyhow::Result;
use log::{debug, info};
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration for the load balancer
#[derive(Clone, Debug)]
pub struct LoadBalancerConfig {
    /// Consecutive losses before releasing ownership
    pub capacity_threshold: u32,
    /// Time to wait before attempting takeover of overdue threads (seconds)
    pub takeover_delay_seconds: i64,
    /// Whether load balancing is enabled
    pub enabled: bool,
}

impl Default for LoadBalancerConfig {
    fn default() -> Self {
        Self {
            capacity_threshold: 5,
            takeover_delay_seconds: 10,
            enabled: true,
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
                debug!(
                    "Thread {} - lost to {} (consecutive losses: {}/{})",
                    thread_pubkey,
                    last_executor,
                    thread_track.consecutive_losses,
                    self.config.capacity_threshold
                );

                // Check if we should release ownership
                if thread_track.consecutive_losses >= self.config.capacity_threshold {
                    info!(
                        "Thread {} - releasing ownership after {} consecutive losses to {}",
                        thread_pubkey, thread_track.consecutive_losses, last_executor
                    );
                    thread_track.owned = false;
                    thread_track.consecutive_losses = 0;
                }
            }
        }

        // Check capacity after updating ownership
        let should_check_capacity =
            thread_track.consecutive_losses >= self.config.capacity_threshold;

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
            info!(
                "Thread {} - attempting TAKEOVER (overdue by {}s, threshold {}s, last_executor: {})",
                thread_pubkey, overdue_seconds, self.config.takeover_delay_seconds, last_executor
            );
            Ok(ProcessDecision::Process)
        } else if at_capacity {
            // We're at capacity - only process critically overdue threads (1.5x takeover delay)
            if is_overdue && overdue_seconds > (self.config.takeover_delay_seconds * 3) / 2 {
                info!(
                    "Thread {} - at capacity but attempting CRITICAL TAKEOVER (overdue by {}s)",
                    thread_pubkey, overdue_seconds
                );
                Ok(ProcessDecision::Process)
            } else {
                debug!("Thread {} - at capacity, skipping", thread_pubkey);
                Ok(ProcessDecision::AtCapacity)
            }
        } else if last_executor.eq(&Pubkey::default()) {
            // No one has executed this thread yet - try to claim it
            info!("Thread {} - no previous executor, claiming", thread_pubkey);
            Ok(ProcessDecision::Process)
        } else {
            // Someone else owns this thread and it's current
            debug!(
                "Thread {} - owned by {}, skipping (overdue: {}, overdue_seconds: {})",
                thread_pubkey, last_executor, is_overdue, overdue_seconds
            );
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
            let was_owned = thread_track.owned;
            thread_track.owned = true;
            thread_track.consecutive_losses = 0;

            // Reset at_capacity if we successfully took ownership of a new thread
            if !was_owned {
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

    /// Remove a thread from tracking when it's deleted
    pub async fn remove_thread(&self, thread_pubkey: &Pubkey) {
        let mut tracking = self.tracking.write().await;
        if tracking.remove(thread_pubkey).is_some() {
            debug!(
                "Removed thread {} from load balancer tracking",
                thread_pubkey
            );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> LoadBalancerConfig {
        LoadBalancerConfig {
            capacity_threshold: 3,
            takeover_delay_seconds: 5,
            enabled: true,
        }
    }

    #[tokio::test]
    async fn test_first_execution_claims_ownership() {
        let executor = Pubkey::new_unique();
        let lb = LoadBalancer::new(executor, test_config());
        let thread = Pubkey::new_unique();

        // No previous executor - should claim
        let decision = lb
            .should_process(&thread, &Pubkey::default(), false, 0)
            .await
            .unwrap();
        assert_eq!(decision, ProcessDecision::Process);
    }

    #[tokio::test]
    async fn test_owned_thread_always_processes() {
        let executor = Pubkey::new_unique();
        let lb = LoadBalancer::new(executor, test_config());
        let thread = Pubkey::new_unique();

        // Record successful execution to claim ownership
        lb.record_execution_result(&thread, true, 1000).await.unwrap();

        // Should process owned thread even if someone else executed last
        let other_executor = Pubkey::new_unique();
        let decision = lb
            .should_process(&thread, &other_executor, false, 0)
            .await
            .unwrap();
        assert_eq!(decision, ProcessDecision::Process);
    }

    #[tokio::test]
    async fn test_ownership_released_after_losses() {
        let executor = Pubkey::new_unique();
        let config = LoadBalancerConfig {
            capacity_threshold: 2,
            takeover_delay_seconds: 5,
            enabled: true,
        };
        let lb = LoadBalancer::new(executor, config);
        let thread = Pubkey::new_unique();

        // Claim ownership
        lb.record_execution_result(&thread, true, 1000).await.unwrap();

        let other_executor = Pubkey::new_unique();

        // First loss - still owned
        let _ = lb
            .should_process(&thread, &other_executor, false, 0)
            .await
            .unwrap();
        let stats = lb.get_stats().await;
        assert_eq!(stats.owned_threads, 1);

        // Second loss - ownership released (threshold is 2)
        let _ = lb
            .should_process(&thread, &other_executor, false, 0)
            .await
            .unwrap();
        let stats = lb.get_stats().await;
        assert_eq!(stats.owned_threads, 0);
    }

    #[tokio::test]
    async fn test_takeover_overdue_threads() {
        let executor = Pubkey::new_unique();
        let lb = LoadBalancer::new(executor, test_config());
        let thread = Pubkey::new_unique();
        let other_executor = Pubkey::new_unique();

        // Thread executed by someone else, not overdue - should skip
        let decision = lb
            .should_process(&thread, &other_executor, false, 0)
            .await
            .unwrap();
        assert_eq!(decision, ProcessDecision::Skip);

        // Thread overdue beyond threshold - should attempt takeover
        let decision = lb
            .should_process(&thread, &other_executor, true, 10)
            .await
            .unwrap();
        assert_eq!(decision, ProcessDecision::Process);
    }

    #[tokio::test]
    async fn test_disabled_always_processes() {
        let executor = Pubkey::new_unique();
        let config = LoadBalancerConfig {
            enabled: false,
            ..Default::default()
        };
        let lb = LoadBalancer::new(executor, config);
        let thread = Pubkey::new_unique();
        let other_executor = Pubkey::new_unique();

        // Should always process when disabled
        let decision = lb
            .should_process(&thread, &other_executor, false, 0)
            .await
            .unwrap();
        assert_eq!(decision, ProcessDecision::Process);
    }
}
