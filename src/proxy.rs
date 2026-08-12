// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashMap, fmt, future::Future, io, sync::Arc, time::Duration};

use serde::Deserialize;
use serde_json::Value;
use tokio::{
    io::{
        AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt,
        BufReader,
    },
    net::{TcpListener, TcpStream},
    sync::{Mutex, Semaphore, mpsc},
    time::timeout,
};

use crate::{
    config::Config,
    protocol::{
        ClientFrame, classify, invalid_frame_response, pending_request_limit_response,
        relay_error_response, success_response,
    },
    relay::{RelayErrorKind, SharedRelay},
};

#[derive(Debug)]
pub struct ProxyError(String);

enum FinishedTask {
    Writer,
    Upstream,
    Client,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingRequest {
    Upstream,
    Broadcast,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Reservation {
    Reserved,
    Duplicate,
    LimitExceeded,
}

type PendingRequests = Arc<Mutex<HashMap<String, PendingRequest>>>;

#[derive(Deserialize)]
struct UpstreamEnvelope {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    method: Option<String>,
}

impl fmt::Display for ProxyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ProxyError {}

impl From<io::Error> for ProxyError {
    fn from(error: io::Error) -> Self {
        Self(error.to_string())
    }
}

/// Runs the proxy until the supplied shutdown future resolves.
///
/// # Errors
///
/// Returns an error when the listener cannot bind or accept a connection.
pub async fn serve_until<F>(
    config: Config,
    relay: SharedRelay,
    shutdown: F,
) -> Result<(), ProxyError>
where
    F: Future<Output = ()>,
{
    let listener = TcpListener::bind(config.listen).await?;
    let connection_slots = Arc::new(Semaphore::new(config.max_connections));
    tokio::pin!(shutdown);

    loop {
        let accepted = tokio::select! {
            result = listener.accept() => Some(result),
            () = &mut shutdown => None,
        };
        let Some(accepted) = accepted else {
            return Ok(());
        };
        let (client, _) = accepted?;

        let Ok(permit) = Arc::clone(&connection_slots).try_acquire_owned() else {
            drop(client);
            continue;
        };

        let task_config = config.clone();
        let task_relay = Arc::clone(&relay);
        tokio::spawn(async move {
            let _permit = permit;
            let upstream = timeout(
                task_config.connect_timeout,
                TcpStream::connect((task_config.upstream.host(), task_config.upstream.port())),
            )
            .await;
            let Ok(Ok(upstream)) = upstream else {
                return;
            };

            let _ = proxy_streams(
                client,
                upstream,
                task_relay,
                task_config.max_frame_bytes,
                task_config.max_pending_requests,
            )
            .await;
        });
    }
}

async fn proxy_streams<C, U>(
    client: C,
    upstream: U,
    relay: SharedRelay,
    max_frame_bytes: usize,
    max_pending_requests: usize,
) -> io::Result<()>
where
    C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    U: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (client_reader, client_writer) = tokio::io::split(client);
    let (upstream_reader, upstream_writer) = tokio::io::split(upstream);
    let (client_output, output) = mpsc::channel::<Vec<u8>>(32);
    let pending_requests = Arc::new(Mutex::new(HashMap::new()));

    let mut writer_task = tokio::spawn(write_client_output(client_writer, output));
    let mut upstream_task = tokio::spawn(forward_upstream_frames(
        upstream_reader,
        client_output.clone(),
        Arc::clone(&pending_requests),
        max_frame_bytes,
    ));
    let mut client_task = tokio::spawn(handle_client_frames(
        client_reader,
        upstream_writer,
        client_output.clone(),
        relay,
        pending_requests,
        max_frame_bytes,
        max_pending_requests,
    ));
    drop(client_output);

    let finished = tokio::select! {
        _ = &mut writer_task => FinishedTask::Writer,
        _ = &mut upstream_task => FinishedTask::Upstream,
        _ = &mut client_task => FinishedTask::Client,
    };

    match finished {
        FinishedTask::Writer => {
            upstream_task.abort();
            client_task.abort();
        }
        FinishedTask::Upstream => {
            client_task.abort();
            let _ = client_task.await;
            let _ = timeout(Duration::from_secs(1), &mut writer_task).await;
            writer_task.abort();
        }
        FinishedTask::Client => {
            upstream_task.abort();
            let _ = upstream_task.await;
            let _ = timeout(Duration::from_secs(1), &mut writer_task).await;
            writer_task.abort();
        }
    }
    Ok(())
}

async fn write_client_output<W>(
    mut writer: W,
    mut output: mpsc::Receiver<Vec<u8>>,
) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    while let Some(frame) = output.recv().await {
        writer.write_all(&frame).await?;
    }
    writer.shutdown().await
}

async fn forward_upstream_frames<R>(
    reader: R,
    client_output: mpsc::Sender<Vec<u8>>,
    pending_requests: PendingRequests,
    max_frame_bytes: usize,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
{
    let mut reader = BufReader::new(reader);
    while let Some(frame) = read_frame(&mut reader, max_frame_bytes).await? {
        let envelope = serde_json::from_slice::<UpstreamEnvelope>(&frame)
            .map_err(|_| invalid_upstream_frame())?;
        let should_forward = match (envelope.id, envelope.method) {
            (Some(id @ (Value::Number(_) | Value::String(_))), None) => {
                let key = request_id_key(&id).expect("validated request ID");
                let mut pending = pending_requests.lock().await;
                match pending.get(&key) {
                    Some(PendingRequest::Upstream) => {
                        pending.remove(&key);
                        true
                    }
                    Some(PendingRequest::Broadcast) | None => false,
                }
            }
            (None, Some(_)) => true,
            _ => return Err(invalid_upstream_frame()),
        };
        if !should_forward {
            continue;
        }
        if client_output.send(frame).await.is_err() {
            return Ok(());
        }
    }
    Ok(())
}

async fn handle_client_frames<R, W>(
    reader: R,
    mut upstream_writer: W,
    client_output: mpsc::Sender<Vec<u8>>,
    relay: SharedRelay,
    pending_requests: PendingRequests,
    max_frame_bytes: usize,
    max_pending_requests: usize,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut reader = BufReader::new(reader);
    loop {
        let frame = match read_frame(&mut reader, max_frame_bytes).await {
            Ok(Some(frame)) => frame,
            Ok(None) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                let _ = client_output.send(invalid_frame_response()).await;
                return Ok(());
            }
            Err(error) => return Err(error),
        };

        match classify(&frame) {
            Ok(ClientFrame::Passthrough { id }) => {
                if let Some(id) = id {
                    let key = request_id_key(&id).expect("validated request ID");
                    match reserve_request(
                        &pending_requests,
                        key,
                        PendingRequest::Upstream,
                        max_pending_requests,
                    )
                    .await
                    {
                        Reservation::Reserved => {}
                        Reservation::Duplicate => {
                            let _ = client_output.send(invalid_frame_response()).await;
                            return Ok(());
                        }
                        Reservation::LimitExceeded => {
                            let _ = client_output
                                .send(pending_request_limit_response(&id))
                                .await;
                            return Ok(());
                        }
                    }
                }
                upstream_writer.write_all(&frame).await?;
            }
            Ok(ClientFrame::Broadcast {
                id,
                raw_transaction,
            }) => {
                let broadcast_key = request_id_key(&id);
                if let Some(key) = broadcast_key.clone() {
                    match reserve_request(
                        &pending_requests,
                        key,
                        PendingRequest::Broadcast,
                        max_pending_requests,
                    )
                    .await
                    {
                        Reservation::Reserved => {}
                        Reservation::Duplicate => {
                            let _ = client_output.send(invalid_frame_response()).await;
                            return Ok(());
                        }
                        Reservation::LimitExceeded => {
                            let _ = client_output
                                .send(pending_request_limit_response(&id))
                                .await;
                            return Ok(());
                        }
                    }
                }
                let response = match relay.broadcast(&raw_transaction).await {
                    Ok(transaction_id) => success_response(&id, &transaction_id),
                    Err(error) => {
                        relay_error_response(&id, error.kind() == RelayErrorKind::Disabled)
                    }
                };
                if let Some(key) = broadcast_key {
                    pending_requests.lock().await.remove(&key);
                }
                if client_output.send(response).await.is_err() {
                    return Ok(());
                }
            }
            Err(error) => {
                let _ = client_output.send(error.response()).await;
                return Ok(());
            }
        }
    }
}

async fn reserve_request(
    pending_requests: &PendingRequests,
    key: String,
    request: PendingRequest,
    max_pending_requests: usize,
) -> Reservation {
    let mut pending = pending_requests.lock().await;
    if pending.contains_key(&key) {
        return Reservation::Duplicate;
    }
    if pending.len() >= max_pending_requests {
        return Reservation::LimitExceeded;
    }
    pending.insert(key, request);
    Reservation::Reserved
}

fn request_id_key(id: &Value) -> Option<String> {
    match id {
        Value::Number(_) | Value::String(_) => serde_json::to_string(id).ok(),
        _ => None,
    }
}

fn invalid_upstream_frame() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid upstream frame")
}

async fn read_frame<R>(reader: &mut R, max_frame_bytes: usize) -> io::Result<Option<Vec<u8>>>
where
    R: AsyncBufRead + Unpin,
{
    let mut frame = Vec::with_capacity(1_024);
    let bytes_read = reader
        .take((max_frame_bytes + 1) as u64)
        .read_until(b'\n', &mut frame)
        .await?;
    if bytes_read == 0 {
        return Ok(None);
    }
    if frame.len() > max_frame_bytes || !frame.ends_with(b"\n") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Electrum frame",
        ));
    }
    Ok(Some(frame))
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader, duplex},
        sync::Notify,
        time::{Duration, timeout},
    };

    use super::proxy_streams;
    use crate::relay::{BroadcastRelay, RejectRelay, RelayError, SharedRelay};

    #[derive(Default)]
    struct MockRelay {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl BroadcastRelay for MockRelay {
        async fn broadcast(&self, _raw_transaction: &str) -> Result<String, RelayError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok("11".repeat(32))
        }
    }

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

    #[tokio::test]
    async fn forwards_queries_but_intercepts_broadcasts() {
        let (wallet, proxy_client) = duplex(16 * 1024);
        let (proxy_upstream, upstream) = duplex(16 * 1024);
        let relay = Arc::new(MockRelay::default());
        let shared_relay: SharedRelay = relay.clone();

        let proxy = tokio::spawn(proxy_streams(
            proxy_client,
            proxy_upstream,
            shared_relay,
            4_096,
            16,
        ));
        let (wallet_reader, mut wallet_writer) = tokio::io::split(wallet);
        let (upstream_reader, mut upstream_writer) = tokio::io::split(upstream);
        let mut wallet_reader = BufReader::new(wallet_reader);
        let mut upstream_reader = BufReader::new(upstream_reader);

        let version_request = b"{\"id\":1,\"method\":\"server.version\",\"params\":[]}\n";
        wallet_writer
            .write_all(version_request)
            .await
            .expect("wallet request");
        let mut observed = Vec::new();
        upstream_reader
            .read_until(b'\n', &mut observed)
            .await
            .expect("upstream request");
        assert_eq!(observed, version_request);

        upstream_writer
            .write_all(b"{\"id\":1,\"result\":[\"upstream\",\"1.4\"]}\n")
            .await
            .expect("upstream response");
        let mut response = String::new();
        wallet_reader
            .read_line(&mut response)
            .await
            .expect("wallet response");
        assert!(response.contains("upstream"));

        wallet_writer
            .write_all(
                b"{\"id\":2,\"method\":\"blockchain.transaction.broadcast\",\"params\":[\"00aa\"]}\n",
            )
            .await
            .expect("broadcast request");
        response.clear();
        wallet_reader
            .read_line(&mut response)
            .await
            .expect("broadcast response");
        assert!(response.contains(&"11".repeat(32)));
        assert_eq!(relay.calls.load(Ordering::SeqCst), 1);

        let mut leaked = Vec::new();
        assert!(
            timeout(
                Duration::from_millis(50),
                upstream_reader.read_until(b'\n', &mut leaked)
            )
            .await
            .is_err()
        );

        drop(wallet_writer);
        drop(upstream_writer);
        proxy.abort();
    }

    #[tokio::test]
    async fn default_relay_rejects_without_leaking_to_upstream() {
        let (wallet, proxy_client) = duplex(16 * 1024);
        let (proxy_upstream, upstream) = duplex(16 * 1024);
        let relay: SharedRelay = Arc::new(RejectRelay);
        let proxy = tokio::spawn(proxy_streams(
            proxy_client,
            proxy_upstream,
            relay,
            4_096,
            16,
        ));
        let (wallet_reader, mut wallet_writer) = tokio::io::split(wallet);
        let (upstream_reader, _upstream_writer) = tokio::io::split(upstream);
        let mut wallet_reader = BufReader::new(wallet_reader);
        let mut upstream_reader = BufReader::new(upstream_reader);

        wallet_writer
            .write_all(
                b"{\"id\":9,\"method\":\"blockchain.transaction.broadcast\",\"params\":[\"00aa\"]}\n",
            )
            .await
            .expect("broadcast request");

        let mut response = String::new();
        wallet_reader
            .read_line(&mut response)
            .await
            .expect("rejection response");
        assert!(response.contains("private broadcasting is not configured"));

        let mut leaked = Vec::new();
        assert!(
            timeout(
                Duration::from_millis(50),
                upstream_reader.read_until(b'\n', &mut leaked)
            )
            .await
            .is_err()
        );

        drop(wallet_writer);
        proxy.abort();
    }

    #[tokio::test]
    async fn drops_upstream_response_that_collides_with_intercepted_broadcast() {
        let (wallet, proxy_client) = duplex(16 * 1024);
        let (proxy_upstream, mut upstream) = duplex(16 * 1024);
        let relay = Arc::new(ControlledRelay::default());
        let shared_relay: SharedRelay = relay.clone();
        let proxy = tokio::spawn(proxy_streams(
            proxy_client,
            proxy_upstream,
            shared_relay,
            4_096,
            16,
        ));
        let (wallet_reader, mut wallet_writer) = tokio::io::split(wallet);
        let mut wallet_reader = BufReader::new(wallet_reader);

        wallet_writer
            .write_all(
                b"{\"id\":12,\"method\":\"blockchain.transaction.broadcast\",\"params\":[\"00aa\"]}\n",
            )
            .await
            .expect("broadcast request");
        relay.started.notified().await;

        upstream
            .write_all(format!("{{\"id\":12,\"result\":\"{}\"}}\n", "22".repeat(32)).as_bytes())
            .await
            .expect("injected upstream response");
        relay.release.notify_one();

        let mut response = String::new();
        wallet_reader
            .read_line(&mut response)
            .await
            .expect("correlated relay response");
        assert!(response.contains(&"11".repeat(32)));
        assert!(!response.contains(&"22".repeat(32)));

        drop(wallet_writer);
        drop(upstream);
        proxy.abort();
    }

    #[tokio::test]
    async fn closes_connection_when_pending_request_window_is_full() {
        let (wallet, proxy_client) = duplex(16 * 1024);
        let (proxy_upstream, upstream) = duplex(16 * 1024);
        let relay: SharedRelay = Arc::new(RejectRelay);
        let proxy = tokio::spawn(proxy_streams(proxy_client, proxy_upstream, relay, 4_096, 2));
        let (wallet_reader, mut wallet_writer) = tokio::io::split(wallet);
        let (upstream_reader, _upstream_writer) = tokio::io::split(upstream);
        let mut wallet_reader = BufReader::new(wallet_reader);
        let mut upstream_reader = BufReader::new(upstream_reader);

        for id in 1..=3 {
            wallet_writer
                .write_all(
                    format!("{{\"id\":{id},\"method\":\"server.version\",\"params\":[]}}\n")
                        .as_bytes(),
                )
                .await
                .expect("wallet request");
        }

        for expected_id in 1..=2 {
            let mut observed = String::new();
            upstream_reader
                .read_line(&mut observed)
                .await
                .expect("accepted upstream request");
            assert!(observed.contains(&format!("\"id\":{expected_id}")));
        }

        let mut response = String::new();
        timeout(
            Duration::from_secs(1),
            wallet_reader.read_line(&mut response),
        )
        .await
        .expect("limit response timeout")
        .expect("limit response");
        assert!(response.contains("\"id\":3"));
        assert!(response.contains("\"code\":-32092"));

        let mut unexpected = String::new();
        let bytes_read = timeout(
            Duration::from_secs(1),
            upstream_reader.read_line(&mut unexpected),
        )
        .await
        .expect("upstream did not close")
        .expect("upstream close");
        assert_eq!(bytes_read, 0);

        proxy.await.expect("proxy task").expect("proxy result");
    }

    #[tokio::test]
    async fn does_not_relay_broadcast_when_pending_request_window_is_full() {
        let (wallet, proxy_client) = duplex(16 * 1024);
        let (proxy_upstream, upstream) = duplex(16 * 1024);
        let relay = Arc::new(MockRelay::default());
        let shared_relay: SharedRelay = relay.clone();
        let proxy = tokio::spawn(proxy_streams(
            proxy_client,
            proxy_upstream,
            shared_relay,
            4_096,
            1,
        ));
        let (wallet_reader, mut wallet_writer) = tokio::io::split(wallet);
        let (upstream_reader, _upstream_writer) = tokio::io::split(upstream);
        let mut wallet_reader = BufReader::new(wallet_reader);
        let mut upstream_reader = BufReader::new(upstream_reader);

        let query = b"{\"id\":1,\"method\":\"server.version\",\"params\":[]}\n";
        wallet_writer.write_all(query).await.expect("wallet query");
        let mut observed = Vec::new();
        upstream_reader
            .read_until(b'\n', &mut observed)
            .await
            .expect("upstream query");
        assert_eq!(observed, query);

        wallet_writer
            .write_all(
                b"{\"id\":2,\"method\":\"blockchain.transaction.broadcast\",\"params\":[\"00aa\"]}\n",
            )
            .await
            .expect("broadcast request");

        let mut response = String::new();
        wallet_reader
            .read_line(&mut response)
            .await
            .expect("limit response");
        assert!(response.contains("\"id\":2"));
        assert!(response.contains("\"code\":-32092"));
        assert_eq!(relay.calls.load(Ordering::SeqCst), 0);

        let mut leaked = Vec::new();
        let bytes_read = upstream_reader
            .read_until(b'\n', &mut leaked)
            .await
            .expect("upstream close");
        assert_eq!(bytes_read, 0);

        proxy.await.expect("proxy task").expect("proxy result");
    }

    #[tokio::test]
    async fn correlated_response_releases_pending_request_slot() {
        let (wallet, proxy_client) = duplex(16 * 1024);
        let (proxy_upstream, upstream) = duplex(16 * 1024);
        let relay: SharedRelay = Arc::new(RejectRelay);
        let proxy = tokio::spawn(proxy_streams(proxy_client, proxy_upstream, relay, 4_096, 1));
        let (wallet_reader, mut wallet_writer) = tokio::io::split(wallet);
        let (upstream_reader, mut upstream_writer) = tokio::io::split(upstream);
        let mut wallet_reader = BufReader::new(wallet_reader);
        let mut upstream_reader = BufReader::new(upstream_reader);

        for id in 1..=2 {
            let request = format!("{{\"id\":{id},\"method\":\"server.version\",\"params\":[]}}\n");
            wallet_writer
                .write_all(request.as_bytes())
                .await
                .expect("wallet request");

            let mut observed = String::new();
            upstream_reader
                .read_line(&mut observed)
                .await
                .expect("upstream request");
            assert_eq!(observed, request);

            upstream_writer
                .write_all(format!("{{\"id\":{id},\"result\":true}}\n").as_bytes())
                .await
                .expect("upstream response");
            let mut response = String::new();
            wallet_reader
                .read_line(&mut response)
                .await
                .expect("wallet response");
            assert!(response.contains(&format!("\"id\":{id}")));
            assert!(response.contains("\"result\":true"));
        }

        drop(wallet_writer);
        drop(upstream_writer);
        proxy.abort();
    }
}
