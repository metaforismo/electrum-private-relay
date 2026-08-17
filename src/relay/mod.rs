// SPDX-License-Identifier: Apache-2.0

mod draining;
mod limited;
mod reject;
mod socks_electrum;

use std::{fmt, sync::Arc};

use async_trait::async_trait;

pub use draining::{DrainingRelay, RelayDrainHandle};
pub use limited::LimitedRelay;
pub use reject::RejectRelay;
pub use socks_electrum::SocksElectrumRelay;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelayErrorKind {
    Disabled,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayError {
    kind: RelayErrorKind,
}

impl RelayError {
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            kind: RelayErrorKind::Disabled,
        }
    }

    #[must_use]
    pub const fn failed() -> Self {
        Self {
            kind: RelayErrorKind::Failed,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> RelayErrorKind {
        self.kind
    }
}

impl fmt::Display for RelayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            RelayErrorKind::Disabled => formatter.write_str("private relay is disabled"),
            RelayErrorKind::Failed => formatter.write_str("private relay failed"),
        }
    }
}

impl std::error::Error for RelayError {}

#[async_trait]
pub trait BroadcastRelay: Send + Sync {
    async fn broadcast(&self, raw_transaction: &str) -> Result<String, RelayError>;
}

pub type SharedRelay = Arc<dyn BroadcastRelay>;
