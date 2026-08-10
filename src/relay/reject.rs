// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use super::{BroadcastRelay, RelayError};

#[derive(Clone, Copy, Debug, Default)]
pub struct RejectRelay;

#[async_trait]
impl BroadcastRelay for RejectRelay {
    async fn broadcast(&self, _raw_transaction: &str) -> Result<String, RelayError> {
        Err(RelayError::disabled())
    }
}
