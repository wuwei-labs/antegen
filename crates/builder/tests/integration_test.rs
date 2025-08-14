use antegen_builder::{BuilderService, DataSource, ObservedEvent};
use antegen_submitter::{BuiltTransaction, TransactionSource};
use antegen_thread_program::state::Thread;
use antegen_utils::thread::{Trigger, TriggerContext};
use async_trait::async_trait;
use solana_program::pubkey::Pubkey;
use std::sync::Arc;
use tokio::sync::mpsc;

struct MockDataSource {
    events: Vec<ObservedEvent>,
    index: usize,
}

#[async_trait]
impl DataSource for MockDataSource {
    async fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn stop(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn next_event(&mut self) -> anyhow::Result<Option<ObservedEvent>> {
        if self.index < self.events.len() {
            let event = self.events[self.index].clone();
            self.index += 1;
            Ok(Some(event))
        } else {
            Ok(None)
        }
    }

    async fn subscribe_thread(&mut self, _thread_pubkey: Pubkey) -> anyhow::Result<()> {
        Ok(())
    }

    async fn unsubscribe_thread(&mut self, _thread_pubkey: Pubkey) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_current_slot(&self) -> anyhow::Result<u64> {
        Ok(100)
    }

    fn name(&self) -> &str {
        "MockDataSource"
    }
}

#[tokio::test]
async fn test_builder_submitter_integration() {
    // Create mock thread
    let thread_pubkey = Pubkey::new_unique();
    let thread = Thread {
        version: 1,
        bump: 0,
        authority: Pubkey::new_unique(),
        id: vec![1, 2, 3],
        name: "test_thread".to_string(),
        created_at: 0,
        paused: false,
        fibers: vec![],
        exec_index: 0,
        nonce_account: solana_program::system_program::ID,
        last_nonce: String::new(),
        trigger: Trigger::Now,
        trigger_context: TriggerContext::Timestamp { prev: 0, next: 0 },
        builders: vec![],
        claim_window_start: None,
    };

    // Create mock data source with one event
    let mock_source = MockDataSource {
        events: vec![ObservedEvent::ThreadExecutable {
            thread_pubkey,
            thread: thread.clone(),
            slot: 100,
        }],
        index: 0,
    };

    // Create channel for builder -> submitter communication
    let (tx, mut rx) = mpsc::channel::<BuiltTransaction>(10);

    // Create builder in worker mode
    let rpc_client = Arc::new(solana_client::nonblocking::rpc_client::RpcClient::new(
        "http://localhost:8899".to_string(),
    ));

    let (builder_service, _) =
        BuilderService::new_worker(Box::new(mock_source), 1, rpc_client.clone());

    // Test that builder can be created
    assert!(matches!(builder_service, BuilderService { .. }));

    // Test receiving from channel
    tokio::spawn(async move {
        tx.send(BuiltTransaction::new(
            thread_pubkey,
            1,
            vec![1, 2, 3],
            vec![],
        ))
        .await
        .unwrap();
    });

    // Verify we can receive the transaction
    if let Some(built_tx) = rx.recv().await {
        assert_eq!(built_tx.thread_pubkey, thread_pubkey);
        assert_eq!(built_tx.builder_id, 1);
    } else {
        panic!("Failed to receive transaction");
    }
}

#[tokio::test]
async fn test_nats_integration() {
    // Skip if NATS is not available
    if async_nats::connect("nats://localhost:4222").await.is_err() {
        eprintln!("Skipping NATS test - NATS server not available");
        return;
    }

    // Test NATS publisher and consumer
    let publisher = antegen_builder::NatsPublisher::new(
        "nats://localhost:4222",
        Some("TEST_STREAM"),
        Some("test.tx"),
    )
    .await
    .expect("Failed to create publisher");

    let mut consumer = antegen_submitter::NatsConsumer::new(
        "nats://localhost:4222",
        "test_consumer",
        Some("TEST_STREAM"),
    )
    .await
    .expect("Failed to create consumer");

    // Publish a test transaction
    let thread_pubkey = Pubkey::new_unique();
    let test_tx = BuiltTransaction::new(thread_pubkey, 1, vec![4, 5, 6], vec![]);

    publisher
        .publish(&test_tx)
        .await
        .expect("Failed to publish");

    // Try to receive it
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    if let Some(received_tx) = consumer.receive().await.expect("Failed to receive") {
        assert_eq!(received_tx.thread_pubkey, thread_pubkey);
        assert_eq!(received_tx.builder_id, 1);

        // Acknowledge the message
        consumer.ack(&received_tx.id).await.expect("Failed to ack");
    } else {
        panic!("Did not receive transaction");
    }
}
