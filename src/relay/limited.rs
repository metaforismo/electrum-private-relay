// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use tokio::sync::Semaphore;

use super::{BroadcastRelay, RelayError, SharedRelay};
use crate::config::{
    ConcurrentBroadcastLimit, MAX_CONFIGURABLE_IN_FLIGHT_BROADCAST_BYTES,
};

/// Process-wide fail-closed limiter for private relay submissions.
///
/// The limiter never queues a broadcast. When all request slots or aggregate
/// raw-transaction payload permits are in use, the request fails immediately
/// and the wrapped relay is not called.
pub struct LimitedRelay {
    inner: SharedRelay,
    requests: Semaphore,
    payload_bytes: Semaphore,
}

impl LimitedRelay {
    #[must_use]
    pub fn new(inner: SharedRelay, limit: ConcurrentBroadcastLimit) -> Self {
        Self::with_payload_budget(
            inner,
            limit,
            MAX_CONFIGURABLE_IN_FLIGHT_BROADCAST_BYTES,
        )
    }

    fn with_payload_budget(
        inner: SharedRelay,
        limit: ConcurrentBroadcastLimit,
        payload_budget: usize,
    ) -> Self {
        Self {
            inner,
            requests: Semaphore::new(limit.get()),
            payload_bytes: Semaphore::new(payload_budget),
        }
    }
}

#[async_trait]
impl BroadcastRelay for LimitedRelay {
    async fn broadcast(&self, raw_transaction: &str) -> Result<String, RelayError> {
        let payload_permits = u32::try_from(raw_transaction.len())
            .map_err(|_| RelayError::failed())?;
        let _request = self
            .requests
            .try_acquire()
            .map_err(|_| RelayError::failed())?;
        let _payload = self
            .payload_bytes
            .try_acquire_many(payload_permits)
            .map_err(|_| RelayError::failed())?;
        self.inner.broadcast(raw_transaction).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use tokio::{
        sync::Notify,
        time::{Duration, timeout},
    };

    use super::*;
    use crate::relay::RelayErrorKind;

    #[derive(Default)]
    struct BlockingRelay {
        calls: AtomicUsize,
        started: Notify,
        release: Notify,
    }

    #[async_trait]
    impl BroadcastRelay for BlockingRelay {
        async fn broadcast(&self, _raw_transaction: &str) -> Result<String, RelayError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            self.release.notified().await;
            Ok("11".repeat(32))
        }
    }

    fn limit(value: usize) -> ConcurrentBroadcastLimit {
        ConcurrentBroadcastLimit::try_from(value)
            .unwrap_or_else(|error| unreachable!("valid limit: {error}"))
    }

    #[tokio::test]
    async fn rejects_excess_broadcast_without_calling_inner_relay() {
        let inner = Arc::new(BlockingRelay::default());
        let shared_inner: SharedRelay = inner.clone();
        let relay = Arc::new(LimitedRelay::new(shared_inner, limit(1)));

        let first = {
            let relay = Arc::clone(&relay);
            tokio::spawn(async move { relay.broadcast("00").await })
        };
        timeout(Duration::from_secs(1), inner.started.notified())
            .await
            .unwrap_or_else(|_| unreachable!("first relay call starts"));

        let second = timeout(Duration::from_secs(1), relay.broadcast("11"))
            .await
            .unwrap_or_else(|_| unreachable!("saturated relay fails immediately"));
        assert!(matches!(
            second,
            Err(error) if error.kind() == RelayErrorKind::Failed
        ));
        assert_eq!(inner.calls.load(Ordering::SeqCst), 1);

        inner.release.notify_one();
        let first_join = timeout(Duration::from_secs(1), first)
            .await
            .unwrap_or_else(|_| unreachable!("first relay call completes"));
        let first_result =
            first_join.unwrap_or_else(|error| unreachable!("relay task joins: {error}"));
        assert!(first_result.is_ok());
    }

    #[tokio::test]
    async fn rejects_aggregate_payload_overload_without_calling_inner_relay() {
        let inner = Arc::new(BlockingRelay::default());
        let shared_inner: SharedRelay = inner.clone();
        let relay = Arc::new(LimitedRelay::with_payload_budget(shared_inner, limit(2), 2));

        let first = {
            let relay = Arc::clone(&relay);
            tokio::spawn(async move { relay.broadcast("00").await })
        };
        timeout(Duration::from_secs(1), inner.started.notified())
            .await
            .unwrap_or_else(|_| unreachable!("first relay call starts"));

        let second = timeout(Duration::from_secs(1), relay.broadcast("11"))
            .await
            .unwrap_or_else(|_| unreachable!("payload overload fails immediately"));
        assert!(matches!(
            second,
            Err(error) if error.kind() == RelayErrorKind::Failed
        ));
        assert_eq!(inner.calls.load(Ordering::SeqCst), 1);

        inner.release.notify_one();
        let first_join = timeout(Duration::from_secs(1), first)
            .await
            .unwrap_or_else(|_| unreachable!("first relay call completes"));
        let first_result =
            first_join.unwrap_or_else(|error| unreachable!("relay task joins: {error}"));
        assert!(first_result.is_ok());
    }
}
