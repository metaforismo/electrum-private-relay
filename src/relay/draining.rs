// SPDX-License-Identifier: Apache-2.0

use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use async_trait::async_trait;
use tokio::{sync::watch, time::timeout};

use super::{BroadcastRelay, RelayError, SharedRelay};

#[derive(Debug)]
struct DrainState {
    accepting: bool,
    active: usize,
}

struct SharedDrainState {
    state: Mutex<DrainState>,
    active: watch::Sender<usize>,
}

impl SharedDrainState {
    fn lock(&self) -> MutexGuard<'_, DrainState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// Handle used by the process lifecycle to reject new relay submissions and
/// wait for already admitted calls to return.
#[derive(Clone)]
pub struct RelayDrainHandle {
    shared: Arc<SharedDrainState>,
}

impl RelayDrainHandle {
    /// Atomically stop admitting new relay calls and return the active count.
    ///
    /// Calls that acquired admission before this method remain active; calls
    /// that race after it fail closed through the ordinary relay error path.
    pub fn begin_shutdown(&self) -> usize {
        let active = {
            let mut state = self.shared.lock();
            state.accepting = false;
            state.active
        };
        self.shared.active.send_replace(active);
        active
    }

    /// Wait until every relay call admitted before shutdown has returned.
    ///
    /// Returns `false` when the bounded wait expires. The caller may then end
    /// the runtime, which cancels any remaining task without attempting another
    /// relay or a public fallback.
    pub async fn wait_for_idle(&self, maximum_wait: Duration) -> bool {
        let mut active = self.shared.active.subscribe();
        if *active.borrow_and_update() == 0 {
            return true;
        }

        timeout(maximum_wait, async {
            loop {
                if active.changed().await.is_err() {
                    return false;
                }
                if *active.borrow_and_update() == 0 {
                    return true;
                }
            }
        })
        .await
        .unwrap_or(false)
    }

    /// Return the current number of admitted relay calls.
    #[must_use]
    pub fn active_calls(&self) -> usize {
        self.shared.lock().active
    }

    /// Return whether new relay calls are still accepted.
    #[must_use]
    pub fn is_accepting(&self) -> bool {
        self.shared.lock().accepting
    }
}

/// Relay wrapper that provides a race-free, fail-closed shutdown boundary.
pub struct DrainingRelay {
    inner: SharedRelay,
    shared: Arc<SharedDrainState>,
}

impl DrainingRelay {
    /// Wrap a relay and return the lifecycle handle controlling admission.
    #[must_use]
    pub fn new(inner: SharedRelay) -> (Self, RelayDrainHandle) {
        let (active, _receiver) = watch::channel(0);
        let shared = Arc::new(SharedDrainState {
            state: Mutex::new(DrainState {
                accepting: true,
                active: 0,
            }),
            active,
        });
        let handle = RelayDrainHandle {
            shared: Arc::clone(&shared),
        };
        (Self { inner, shared }, handle)
    }
}

struct ActiveRelayCall {
    shared: Arc<SharedDrainState>,
}

impl Drop for ActiveRelayCall {
    fn drop(&mut self) {
        let active = {
            let mut state = self.shared.lock();
            if state.active > 0 {
                state.active -= 1;
            }
            state.active
        };
        self.shared.active.send_replace(active);
    }
}

#[async_trait]
impl BroadcastRelay for DrainingRelay {
    async fn broadcast(&self, raw_transaction: &str) -> Result<String, RelayError> {
        let active = {
            let mut state = self.shared.lock();
            if !state.accepting {
                return Err(RelayError::failed());
            }
            state.active = state
                .active
                .checked_add(1)
                .ok_or_else(RelayError::failed)?;
            state.active
        };
        self.shared.active.send_replace(active);
        let _active_call = ActiveRelayCall {
            shared: Arc::clone(&self.shared),
        };
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
    struct ImmediateRelay {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl BroadcastRelay for ImmediateRelay {
        async fn broadcast(&self, _raw_transaction: &str) -> Result<String, RelayError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok("11".repeat(32))
        }
    }

    #[derive(Default)]
    struct ControlledRelay {
        calls: AtomicUsize,
        started: Notify,
        release: Notify,
    }

    #[async_trait]
    impl BroadcastRelay for ControlledRelay {
        async fn broadcast(&self, _raw_transaction: &str) -> Result<String, RelayError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            self.release.notified().await;
            Ok("22".repeat(32))
        }
    }

    #[tokio::test]
    async fn shutdown_rejects_new_calls_without_invoking_inner_relay() {
        let inner = Arc::new(ImmediateRelay::default());
        let shared_inner: SharedRelay = inner.clone();
        let (relay, drain) = DrainingRelay::new(shared_inner);

        assert_eq!(drain.begin_shutdown(), 0);
        assert!(!drain.is_accepting());
        let result = relay.broadcast("00").await;
        assert!(matches!(
            result,
            Err(error) if error.kind() == RelayErrorKind::Failed
        ));
        assert_eq!(inner.calls.load(Ordering::SeqCst), 0);
        assert!(drain.wait_for_idle(Duration::from_millis(10)).await);
    }

    #[tokio::test]
    async fn shutdown_waits_for_an_already_admitted_call() {
        let inner = Arc::new(ControlledRelay::default());
        let shared_inner: SharedRelay = inner.clone();
        let (relay, drain) = DrainingRelay::new(shared_inner);
        let relay = Arc::new(relay);

        let active = {
            let relay = Arc::clone(&relay);
            tokio::spawn(async move { relay.broadcast("00").await })
        };
        timeout(Duration::from_secs(1), inner.started.notified())
            .await
            .unwrap_or_else(|_| unreachable!("relay call starts"));
        assert_eq!(drain.begin_shutdown(), 1);
        assert_eq!(drain.active_calls(), 1);
        assert!(
            timeout(Duration::from_millis(20), drain.wait_for_idle(Duration::from_secs(1)))
                .await
                .is_err()
        );

        inner.release.notify_one();
        assert!(drain.wait_for_idle(Duration::from_secs(1)).await);
        let result = active
            .await
            .unwrap_or_else(|error| unreachable!("relay task joins: {error}"));
        assert!(result.is_ok());
        assert_eq!(drain.active_calls(), 0);
    }

    #[tokio::test]
    async fn bounded_wait_expires_for_a_stuck_relay() {
        let inner = Arc::new(ControlledRelay::default());
        let shared_inner: SharedRelay = inner.clone();
        let (relay, drain) = DrainingRelay::new(shared_inner);
        let relay = Arc::new(relay);

        let active = {
            let relay = Arc::clone(&relay);
            tokio::spawn(async move { relay.broadcast("00").await })
        };
        timeout(Duration::from_secs(1), inner.started.notified())
            .await
            .unwrap_or_else(|_| unreachable!("relay call starts"));
        drain.begin_shutdown();

        assert!(!drain.wait_for_idle(Duration::from_millis(20)).await);
        assert_eq!(drain.active_calls(), 1);
        active.abort();
        let _ = active.await;
        assert!(drain.wait_for_idle(Duration::from_secs(1)).await);
    }
}
