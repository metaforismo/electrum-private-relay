// SPDX-License-Identifier: Apache-2.0

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener as StdTcpListener},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use electrum_private_relay::{
    config::{ConcurrentBroadcastLimit, Config, PendingRequestLimit, RelayMode},
    endpoint::Endpoint,
    proxy::serve_until,
    relay::{BroadcastRelay, DrainingRelay, RelayError, SharedRelay},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::{Notify, oneshot},
    time::timeout,
};

#[derive(Default)]
struct ControlledRelay {
    started: Notify,
    release: Notify,
}

#[async_trait]
impl BroadcastRelay for ControlledRelay {
    async fn broadcast(&self, _raw_transaction: &str) -> Result<String, RelayError> {
        self.started.notify_one();
        self.release.notified().await;
        Ok("11".repeat(32))
    }
}

fn unused_loopback_address() -> SocketAddr {
    let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap_or_else(|error| unreachable!("ephemeral listener binds: {error}"));
    listener
        .local_addr()
        .unwrap_or_else(|error| unreachable!("ephemeral address exists: {error}"))
}

async fn connect_with_retry(address: SocketAddr) -> TcpStream {
    timeout(Duration::from_secs(2), async {
        loop {
            match TcpStream::connect(address).await {
                Ok(stream) => return stream,
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .unwrap_or_else(|_| unreachable!("proxy starts accepting connections"))
}

#[tokio::test]
async fn active_broadcast_completes_after_listener_shutdown() {
    let upstream_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap_or_else(|error| unreachable!("upstream listener binds: {error}"));
    let upstream_address = upstream_listener
        .local_addr()
        .unwrap_or_else(|error| unreachable!("upstream address exists: {error}"));
    let proxy_address = unused_loopback_address();

    let config = Config {
        listen: proxy_address,
        upstream: Endpoint::new(upstream_address.ip().to_string(), upstream_address.port()),
        relay_mode: RelayMode::Reject,
        relay_endpoint: None,
        socks5_proxy: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9_050),
        max_connections: 4,
        max_concurrent_broadcasts: ConcurrentBroadcastLimit::try_from(2)
            .unwrap_or_else(|error| unreachable!("valid broadcast limit: {error}")),
        max_pending_requests: PendingRequestLimit::try_from(16)
            .unwrap_or_else(|error| unreachable!("valid pending limit: {error}")),
        max_frame_bytes: 4_096,
        connect_timeout: Duration::from_secs(1),
        relay_timeout: Duration::from_secs(1),
    };

    let controlled = Arc::new(ControlledRelay::default());
    let inner: SharedRelay = controlled.clone();
    let (relay, drain) = DrainingRelay::new(inner);
    let relay: SharedRelay = Arc::new(relay);
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = tokio::spawn(serve_until(config, relay, async {
        let _ = shutdown_receiver.await;
    }));

    let wallet = connect_with_retry(proxy_address).await;
    let (upstream, _) = timeout(Duration::from_secs(1), upstream_listener.accept())
        .await
        .unwrap_or_else(|_| unreachable!("proxy connects to upstream"))
        .unwrap_or_else(|error| unreachable!("upstream accepts: {error}"));
    let mut upstream_reader = BufReader::new(upstream);
    let (wallet_reader, mut wallet_writer) = tokio::io::split(wallet);
    let mut wallet_reader = BufReader::new(wallet_reader);

    wallet_writer
        .write_all(
            b"{\"id\":7,\"method\":\"blockchain.transaction.broadcast\",\"params\":[\"00aa\"]}\n",
        )
        .await
        .unwrap_or_else(|error| unreachable!("wallet sends broadcast: {error}"));
    timeout(Duration::from_secs(1), controlled.started.notified())
        .await
        .unwrap_or_else(|_| unreachable!("relay call starts"));

    let mut leaked_upstream = String::new();
    assert!(
        timeout(
            Duration::from_millis(100),
            upstream_reader.read_line(&mut leaked_upstream)
        )
        .await
        .is_err(),
        "intercepted broadcast must not reach the query upstream"
    );
    assert!(leaked_upstream.is_empty());

    assert_eq!(drain.begin_shutdown(), 1);
    shutdown_sender
        .send(())
        .unwrap_or_else(|_| unreachable!("server still waits for shutdown"));
    timeout(Duration::from_secs(1), server)
        .await
        .unwrap_or_else(|_| unreachable!("listener stops promptly"))
        .unwrap_or_else(|error| unreachable!("server task joins: {error}"))
        .unwrap_or_else(|error| unreachable!("server exits cleanly: {error}"));
    assert!(!drain.wait_for_idle(Duration::from_millis(20)).await);

    controlled.release.notify_one();
    assert!(drain.wait_for_idle(Duration::from_secs(1)).await);

    let mut response = String::new();
    timeout(
        Duration::from_secs(1),
        wallet_reader.read_line(&mut response),
    )
    .await
    .unwrap_or_else(|_| unreachable!("wallet receives drained relay response"))
    .unwrap_or_else(|error| unreachable!("wallet response reads: {error}"));
    assert!(response.contains("\"id\":7"));
    assert!(response.contains(&"11".repeat(32)));
}
