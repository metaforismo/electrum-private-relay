// SPDX-License-Identifier: Apache-2.0

use std::{net::SocketAddr, time::Duration};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    time::timeout,
};
use tokio_socks::tcp::Socks5Stream;

use super::{BroadcastRelay, RelayError};
use crate::endpoint::Endpoint;

const RELAY_REQUEST_ID: &str = "electrum-private-relay";
const MAX_RELAY_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
struct RelayResponse {
    id: Value,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Clone, Debug)]
pub struct SocksElectrumRelay {
    socks5_proxy: SocketAddr,
    endpoint: Endpoint,
    timeout: Duration,
}

impl SocksElectrumRelay {
    #[must_use]
    pub const fn new(socks5_proxy: SocketAddr, endpoint: Endpoint, timeout: Duration) -> Self {
        Self {
            socks5_proxy,
            endpoint,
            timeout,
        }
    }
}

#[async_trait]
impl BroadcastRelay for SocksElectrumRelay {
    async fn broadcast(&self, raw_transaction: &str) -> Result<String, RelayError> {
        let target = (self.endpoint.host(), self.endpoint.port());
        let stream = timeout(
            self.timeout,
            Socks5Stream::connect(self.socks5_proxy, target),
        )
        .await
        .map_err(|_| RelayError::failed())?
        .map_err(|_| RelayError::failed())?;

        let (reader, mut writer) = tokio::io::split(stream);
        let mut request = serde_json::to_vec(&json!({
            "id": RELAY_REQUEST_ID,
            "method": "blockchain.transaction.broadcast",
            "params": [raw_transaction],
        }))
        .map_err(|_| RelayError::failed())?;
        request.push(b'\n');

        timeout(self.timeout, writer.write_all(&request))
            .await
            .map_err(|_| RelayError::failed())?
            .map_err(|_| RelayError::failed())?;

        let mut reader = BufReader::new(reader);
        let mut response = Vec::new();
        let byte_count = timeout(
            self.timeout,
            (&mut reader)
                .take((MAX_RELAY_RESPONSE_BYTES + 1) as u64)
                .read_until(b'\n', &mut response),
        )
        .await
        .map_err(|_| RelayError::failed())?
        .map_err(|_| RelayError::failed())?;
        if byte_count == 0
            || response.len() > MAX_RELAY_RESPONSE_BYTES
            || !response.ends_with(b"\n")
        {
            return Err(RelayError::failed());
        }

        let response: RelayResponse =
            serde_json::from_slice(&response).map_err(|_| RelayError::failed())?;
        if response.id.as_str() != Some(RELAY_REQUEST_ID) || response.error.is_some() {
            return Err(RelayError::failed());
        }

        let transaction_id = response
            .result
            .as_ref()
            .and_then(Value::as_str)
            .filter(|candidate| {
                candidate.len() == 64 && candidate.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            .ok_or_else(RelayError::failed)?;

        Ok(transaction_id.to_ascii_lowercase())
    }
}
