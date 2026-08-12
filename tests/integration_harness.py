#!/usr/bin/env python3
"""Standard-library helpers for black-box Electrum proxy integration tests."""

from __future__ import annotations

import json
import select
import socket
import subprocess
import threading
import time
from collections.abc import Callable
from pathlib import Path
from typing import Any


JsonObject = dict[str, Any]
Responder = Callable[[JsonObject], JsonObject | None]


def receive_exact(stream: socket.socket, size: int) -> bytes:
    data = bytearray()
    while len(data) < size:
        chunk = stream.recv(size - len(data))
        if not chunk:
            raise RuntimeError("unexpected end of stream")
        data.extend(chunk)
    return bytes(data)


class ElectrumLineServer:
    """Small newline-delimited JSON server with request recording."""

    def __init__(self, responder: Responder) -> None:
        self._responder = responder
        self._listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self._listener.bind(("127.0.0.1", 0))
        self._listener.listen()
        self._listener.settimeout(0.2)
        self.port = self._listener.getsockname()[1]
        self.requests: list[JsonObject] = []
        self._errors: list[BaseException] = []
        self._stop = threading.Event()
        self._lock = threading.Lock()
        self._connections: list[socket.socket] = []
        self._workers: list[threading.Thread] = []
        self._thread = threading.Thread(target=self._accept, daemon=True)
        self._thread.start()

    def _accept(self) -> None:
        while not self._stop.is_set():
            try:
                connection, _ = self._listener.accept()
            except TimeoutError:
                continue
            except OSError:
                if self._stop.is_set():
                    break
                raise
            connection.settimeout(120)
            self._connections.append(connection)
            worker = threading.Thread(
                target=self._handle,
                args=(connection,),
                daemon=True,
            )
            self._workers.append(worker)
            worker.start()

    def _handle(self, connection: socket.socket) -> None:
        try:
            reader = connection.makefile("rb")
            while not self._stop.is_set():
                line = reader.readline()
                if not line:
                    return
                request = json.loads(line)
                if not isinstance(request, dict):
                    raise RuntimeError("expected an Electrum request object")
                with self._lock:
                    self.requests.append(request)
                response = self._responder(request)
                if response is not None:
                    encoded = json.dumps(
                        response,
                        separators=(",", ":"),
                        sort_keys=True,
                    ).encode("utf-8") + b"\n"
                    connection.sendall(encoded)
        except (BrokenPipeError, ConnectionResetError, TimeoutError):
            return
        except BaseException as error:  # surfaced by assert_healthy
            self._errors.append(error)
        finally:
            connection.close()

    def methods(self) -> list[str]:
        with self._lock:
            return [
                str(request.get("method"))
                for request in self.requests
                if request.get("method") is not None
            ]

    def assert_healthy(self) -> None:
        if self._errors:
            raise AssertionError(f"Electrum test server failed: {self._errors[0]}")

    def close(self) -> None:
        self._stop.set()
        self._listener.close()
        for connection in self._connections:
            connection.close()
        self._thread.join(timeout=2)
        for worker in self._workers:
            worker.join(timeout=2)
        self.assert_healthy()

    def __enter__(self) -> ElectrumLineServer:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()


class Socks5Forwarder:
    """A no-auth SOCKS5 server that records names and forwards to one test relay."""

    def __init__(self, relay_port: int) -> None:
        self._relay_port = relay_port
        self._listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self._listener.bind(("127.0.0.1", 0))
        self._listener.listen()
        self._listener.settimeout(0.2)
        self.port = self._listener.getsockname()[1]
        self.destinations: list[tuple[str, int]] = []
        self._errors: list[BaseException] = []
        self._stop = threading.Event()
        self._connections: list[socket.socket] = []
        self._workers: list[threading.Thread] = []
        self._thread = threading.Thread(target=self._accept, daemon=True)
        self._thread.start()

    def _accept(self) -> None:
        while not self._stop.is_set():
            try:
                connection, _ = self._listener.accept()
            except TimeoutError:
                continue
            except OSError:
                if self._stop.is_set():
                    break
                raise
            self._connections.append(connection)
            worker = threading.Thread(
                target=self._handle,
                args=(connection,),
                daemon=True,
            )
            self._workers.append(worker)
            worker.start()

    def _handle(self, client: socket.socket) -> None:
        relay: socket.socket | None = None
        try:
            version, method_count = receive_exact(client, 2)
            methods = receive_exact(client, method_count)
            if version != 5 or 0 not in methods:
                raise RuntimeError("SOCKS client did not offer no-auth SOCKS5")
            client.sendall(b"\x05\x00")

            version, command, reserved, address_type = receive_exact(client, 4)
            if (version, command, reserved) != (5, 1, 0):
                raise RuntimeError("SOCKS client did not issue CONNECT")
            if address_type == 1:
                host = socket.inet_ntoa(receive_exact(client, 4))
            elif address_type == 3:
                host_length = receive_exact(client, 1)[0]
                host = receive_exact(client, host_length).decode("ascii")
            elif address_type == 4:
                host = socket.inet_ntop(socket.AF_INET6, receive_exact(client, 16))
            else:
                raise RuntimeError(f"unsupported SOCKS address type {address_type}")
            port = int.from_bytes(receive_exact(client, 2), "big")
            self.destinations.append((host, port))

            relay = socket.create_connection(("127.0.0.1", self._relay_port), timeout=5)
            client.sendall(b"\x05\x00\x00\x01\x7f\x00\x00\x01\x00\x00")
            self._copy_bidirectionally(client, relay)
        except (BrokenPipeError, ConnectionResetError, TimeoutError):
            return
        except BaseException as error:  # surfaced by assert_healthy
            self._errors.append(error)
        finally:
            client.close()
            if relay is not None:
                relay.close()

    def _copy_bidirectionally(self, first: socket.socket, second: socket.socket) -> None:
        streams = [first, second]
        while not self._stop.is_set():
            readable, _, _ = select.select(streams, [], [], 0.2)
            for source in readable:
                data = source.recv(64 * 1024)
                if not data:
                    return
                destination = second if source is first else first
                destination.sendall(data)

    def assert_healthy(self) -> None:
        if self._errors:
            raise AssertionError(f"SOCKS5 test forwarder failed: {self._errors[0]}")

    def close(self) -> None:
        self._stop.set()
        self._listener.close()
        for connection in self._connections:
            connection.close()
        self._thread.join(timeout=2)
        for worker in self._workers:
            worker.join(timeout=2)
        self.assert_healthy()

    def __enter__(self) -> Socks5Forwarder:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()


def unused_port() -> int:
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    port = listener.getsockname()[1]
    listener.close()
    return port


class RunningProxy:
    def __init__(
        self,
        binary: Path,
        query_port: int,
        relay_port: int,
        socks_port: int,
        relay_host: str = "regtest-relay.invalid",
        relay_timeout_seconds: int = 10,
    ) -> None:
        self.port = unused_port()
        self._socket_timeout = relay_timeout_seconds + 5
        self._process = subprocess.Popen(
            [
                str(binary),
                "--listen",
                f"127.0.0.1:{self.port}",
                "--upstream",
                f"127.0.0.1:{query_port}",
                "--relay-mode",
                "socks-electrum",
                "--relay-endpoint",
                f"{relay_host}:{relay_port}",
                "--socks5-proxy",
                f"127.0.0.1:{socks_port}",
                "--max-frame-bytes",
                str(4 * 1024 * 1024),
                "--relay-timeout-seconds",
                str(relay_timeout_seconds),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

    def connect(self) -> WalletConnection:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if self._process.poll() is not None:
                stdout, stderr = self._process.communicate()
                raise RuntimeError(
                    f"proxy exited early ({self._process.returncode}): {stdout}{stderr}"
                )
            try:
                stream = socket.create_connection(("127.0.0.1", self.port), timeout=0.2)
                stream.settimeout(self._socket_timeout)
                return WalletConnection(stream)
            except OSError:
                time.sleep(0.025)
        raise RuntimeError("proxy listener did not become ready")

    def close(self) -> None:
        if self._process.poll() is None:
            self._process.terminate()
            try:
                self._process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self._process.kill()
                self._process.wait(timeout=3)
        if self._process.returncode not in (0, -15):
            stdout, stderr = self._process.communicate()
            raise AssertionError(
                f"proxy exited unexpectedly ({self._process.returncode}): {stdout}{stderr}"
            )

    def __enter__(self) -> RunningProxy:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()


class WalletConnection:
    def __init__(self, stream: socket.socket) -> None:
        self._stream = stream
        self._reader = stream.makefile("rb")

    def request(self, payload: JsonObject) -> JsonObject:
        encoded = json.dumps(payload, separators=(",", ":")).encode("utf-8") + b"\n"
        self._stream.sendall(encoded)
        line = self._reader.readline()
        if not line:
            raise RuntimeError("proxy closed before returning an Electrum response")
        response = json.loads(line)
        if not isinstance(response, dict):
            raise RuntimeError("expected an Electrum response object")
        return response

    def close(self) -> None:
        self._reader.close()
        self._stream.close()

    def __enter__(self) -> WalletConnection:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()


def standard_query_response(request: JsonObject) -> JsonObject:
    method = request.get("method")
    if method == "server.version":
        result: Any = ["electrum-private-relay-test", "1.4"]
    elif method == "blockchain.headers.subscribe":
        result = {"height": 101, "hex": "00" * 80}
    elif method == "server.features":
        result = {"protocol_min": "1.4", "protocol_max": "1.4.2"}
    elif method == "server.ping":
        result = None
    else:
        result = True
    return {"id": request.get("id"), "result": result}


def successful_relay_response(transaction_id: str) -> Responder:
    def respond(request: JsonObject) -> JsonObject:
        if request.get("method") != "blockchain.transaction.broadcast":
            raise RuntimeError("test relay received a non-broadcast request")
        return {"id": request.get("id"), "result": transaction_id}

    return respond


def wait_for(predicate: Callable[[], bool], description: str, timeout: float = 5) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.025)
    raise AssertionError(f"timed out waiting for {description}")
