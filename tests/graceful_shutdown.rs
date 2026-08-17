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

fn test_config(
    proxy_address: SocketAddr,
    upstream_address: SocketAddr,
    relay_timeout: Duration,
) -> Config {
    Config {
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
        relay_timeout,
    }
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

async fn accept_upstream(listener: &TcpListener) -> BufReader<TcpStream> {
    let (upstream, _) = timeout(Duration::from_secs(1), listener.accept())
        .await
        .unwrap_or_else(|_| unreachable!("proxy connects to upstream"))
        .unwrap_or_else(|error| unreachable!("upstream accepts: {error}"));
    BufReader::new(upstream)
}

async fn assert_no_upstream_frame(upstream: &mut BufReader<TcpStream>) {
    let mut leaked = String::new();
    assert!(
        timeout(Duration::from_millis(100), upstream.read_line(&mut leaked))
            .await
            .is_err(),
        "intercepted broadcast must not reach the query upstream"
    );
    assert!(leaked.is_empty());
}

#[tokio::test]
async fn active_broadcast_finishes_and_flushes_before_supervised_shutdown_returns() {
    let upstream_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap_or_else(|error| unreachable!("upstream listener binds: {error}"));
    let upstream_address = upstream_listener
        .local_addr()
        .unwrap_or_else(|error| unreachable!("upstream address exists: {error}"));
    let proxy_address = unused_loopback_address();
    let config = test_config(proxy_address, upstream_address, Duration::from_secs(1));

    let controlled = Arc::new(ControlledRelay::default());
    let inner: SharedRelay = controlled.clone();
    let (relay, drain) = DrainingRelay::new(inner);
    let relay: SharedRelay = Arc::new(relay);
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let mut server = tokio::spawn(serve_until(config, relay, async {
        let _ = shutdown_receiver.await;
    }));

    let wallet = connect_with_retry(proxy_address).await;
    let mut upstream_reader = accept_upstream(&upstream_listener).await;
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
    assert_no_upstream_frame(&mut upstream_reader).await;

    assert_eq!(drain.begin_shutdown(), 1);
    shutdown_sender
        .send(())
        .unwrap_or_else(|()| unreachable!("server still waits for shutdown"));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        !server.is_finished(),
        "server must supervise the active connection instead of detaching it"
    );

    controlled.release.notify_one();
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

    let report = timeout(Duration::from_secs(2), &mut server)
        .await
        .unwrap_or_else(|_| unreachable!("supervised server shutdown is bounded"))
        .unwrap_or_else(|error| unreachable!("server task joins: {error}"))
        .unwrap_or_else(|error| unreachable!("server exits cleanly: {error}"));
    assert!(report.all_connections_drained());
    assert!(drain.wait_for_idle(Duration::from_millis(20)).await);
}

#[tokio::test]
async fn stuck_broadcast_is_cancelled_after_the_bounded_connection_deadline() {
    let upstream_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap_or_else(|error| unreachable!("upstream listener binds: {error}"));
    let upstream_address = upstream_listener
        .local_addr()
        .unwrap_or_else(|error| unreachable!("upstream address exists: {error}"));
    let proxy_address = unused_loopback_address();
    let config = test_config(
        proxy_address,
        upstream_address,
        Duration::from_millis(20),
    );

    let controlled = Arc::new(ControlledRelay::default());
    let inner: SharedRelay = controlled.clone();
    let (relay, drain) = DrainingRelay::new(inner);
    let relay: SharedRelay = Arc::new(relay);
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = tokio::spawn(serve_until(config, relay, async {
        let _ = shutdown_receiver.await;
    }));

    let wallet = connect_with_retry(proxy_address).await;
    let mut upstream_reader = accept_upstream(&upstream_listener).await;
    let (wallet_reader, mut wallet_writer) = tokio::io::split(wallet);
    let mut wallet_reader = BufReader::new(wallet_reader);

    wallet_writer
        .write_all(
            b"{\"id\":8,\"method\":\"blockchain.transaction.broadcast\",\"params\":[\"00aa\"]}\n",
        )
        .await
        .unwrap_or_else(|error| unreachable!("wallet sends broadcast: {error}"));
    timeout(Duration::from_secs(1), controlled.started.notified())
        .await
        .unwrap_or_else(|_| unreachable!("relay call starts"));
    assert_no_upstream_frame(&mut upstream_reader).await;

    assert_eq!(drain.begin_shutdown(), 1);
    shutdown_sender
        .send(())
        .unwrap_or_else(|()| unreachable!("server still waits for shutdown"));
    let report = timeout(Duration::from_secs(2), server)
        .await
        .unwrap_or_else(|_| unreachable!("forced connection shutdown is bounded"))
        .unwrap_or_else(|error| unreachable!("server task joins: {error}"))
        .unwrap_or_else(|error| unreachable!("server exits cleanly: {error}"));
    assert!(!report.all_connections_drained());
    assert!(drain.wait_for_idle(Duration::from_millis(100)).await);

    let mut response = String::new();
    let bytes_read = timeout(
        Duration::from_secs(1),
        wallet_reader.read_line(&mut response),
    )
    .await
    .unwrap_or_else(|_| unreachable!("wallet connection closes after forced shutdown"))
    .unwrap_or_else(|error| unreachable!("wallet close reads: {error}"));
    assert_eq!(bytes_read, 0);
    assert!(response.is_empty());
}

#[tokio::test]
async fn idle_connection_closes_without_consuming_the_shutdown_deadline() {
    let upstream_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap_or_else(|error| unreachable!("upstream listener binds: {error}"));
    let upstream_address = upstream_listener
        .local_addr()
        .unwrap_or_else(|error| unreachable!("upstream address exists: {error}"));
    let proxy_address = unused_loopback_address();
    let config = test_config(proxy_address, upstream_address, Duration::from_secs(1));

    let controlled = Arc::new(ControlledRelay::default());
    let inner: SharedRelay = controlled;
    let (relay, drain) = DrainingRelay::new(inner);
    let relay: SharedRelay = Arc::new(relay);
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server = tokio::spawn(serve_until(config, relay, async {
        let _ = shutdown_receiver.await;
    }));

    let wallet = connect_with_retry(proxy_address).await;
    let _upstream_reader = accept_upstream(&upstream_listener).await;
    let mut wallet_reader = BufReader::new(wallet);

    assert_eq!(drain.begin_shutdown(), 0);
    shutdown_sender
        .send(())
        .unwrap_or_else(|()| unreachable!("server still waits for shutdown"));
    let report = timeout(Duration::from_millis(500), server)
        .await
        .unwrap_or_else(|_| unreachable!("idle connection closes promptly"))
        .unwrap_or_else(|error| unreachable!("server task joins: {error}"))
        .unwrap_or_else(|error| unreachable!("server exits cleanly: {error}"));
    assert!(report.all_connections_drained());

    let mut response = String::new();
    let bytes_read = timeout(
        Duration::from_secs(1),
        wallet_reader.read_line(&mut response),
    )
    .await
    .unwrap_or_else(|_| unreachable!("idle wallet observes connection close"))
    .unwrap_or_else(|error| unreachable!("wallet close reads: {error}"));
    assert_eq!(bytes_read, 0);
    assert!(response.is_empty());
}
