// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use tokio::sync::Semaphore;

use super::{BroadcastRelay, RelayError, SharedRelay};
use crate::config::ConcurrentBroadcastLimit;

/// Process-wide fail-closed limiter for private relay submissions.
///
/// The limiter never queues a broadcast. When all permits are in use, the
/// request fails immediately and the wrapped relay is not called.
pub struct LimitedRelay {
    inner: SharedRelay,
    permits: Semaphore,
}

impl LimitedRelay {
    #[must_use]
    pub fn new(inner: SharedRelay, limit: ConcurrentBroadcastLimit) -> Self {
        Self {
            inner,
            permits: Semaphore::new(limit.get()),
        }
    }
}

#[async_trait]
impl BroadcastRelay for LimitedRelay {
    async fn broadcast(&self, raw_transaction: &str) -> Result<String, RelayError> {
        let _permit = self
            .permits
            .try_acquire()
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

    #[tokio::test]
    async fn rejects_excess_broadcast_without_calling_inner_relay() {
        let inner = Arc::new(BlockingRelay::default());
        let shared_inner: SharedRelay = inner.clone();
        let limit = ConcurrentBroadcastLimit::try_from(1)
            .unwrap_or_else(|error| unreachable!("valid limit: {error}"));
        let relay = Arc::new(LimitedRelay::new(shared_inner, limit));

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
}
