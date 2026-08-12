#!/usr/bin/env python3
"""Broadcast a real signed regtest transaction through the complete proxy path."""

from __future__ import annotations

import argparse
import base64
import json
import os
import secrets
import socket
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any

from integration_harness import (
    ElectrumLineServer,
    RunningProxy,
    Socks5Forwarder,
    standard_query_response,
    wait_for,
)


BITCOIN_CORE_IMAGE = (
    "bitcoin/bitcoin:30.2@"
    "sha256:2b1d8a8d23c67b426afa8cd5b3ffd66b8c4ebbbceef3e214df8627aaead1f517"
)
RPC_USER = "epr-regtest"


def command(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        check=check,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


class BitcoinCoreRegtest:
    def __init__(self, image: str) -> None:
        self.image = image
        self.name = f"epr-regtest-{os.getpid()}-{secrets.token_hex(4)}"
        self.rpc_port = self._unused_port()
        self.rpc_password = secrets.token_hex(24)

    @staticmethod
    def _unused_port() -> int:
        stream = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        stream.bind(("127.0.0.1", 0))
        port = stream.getsockname()[1]
        stream.close()
        return port

    def start(self) -> None:
        command(
            "docker",
            "run",
            "--detach",
            "--rm",
            "--name",
            self.name,
            "--publish",
            f"127.0.0.1:{self.rpc_port}:18443",
            self.image,
            "-regtest=1",
            "-server=1",
            "-listen=0",
            "-dnsseed=0",
            "-fixedseeds=0",
            "-fallbackfee=0.0002",
            f"-rpcuser={RPC_USER}",
            f"-rpcpassword={self.rpc_password}",
            "-rpcallowip=0.0.0.0/0",
            "-rpcbind=0.0.0.0:18443",
            "-printtoconsole=1",
        )
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            try:
                self.rpc("getblockchaininfo")
                return
            except (ConnectionError, RuntimeError, TimeoutError, urllib.error.URLError):
                pass
            time.sleep(0.1)
        logs = command("docker", "logs", self.name, check=False)
        raise RuntimeError(f"Bitcoin Core RPC did not become ready:\n{logs.stdout}{logs.stderr}")

    def rpc(self, method: str, *params: Any, wallet: str | None = None) -> Any:
        wallet_path = ""
        if wallet is not None:
            wallet_path = f"/wallet/{urllib.parse.quote(wallet, safe='')}"
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.rpc_port}{wallet_path}",
            data=json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": "electrum-private-relay-regtest",
                    "method": method,
                    "params": params,
                }
            ).encode("utf-8"),
            headers={
                "Authorization": "Basic "
                + base64.b64encode(
                    f"{RPC_USER}:{self.rpc_password}".encode("utf-8")
                ).decode("ascii"),
                "Content-Type": "application/json",
            },
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=5) as response:
                payload = json.load(response)
        except urllib.error.HTTPError as error:
            payload = json.loads(error.read())
        if payload.get("error") is not None:
            raise RuntimeError(f"Bitcoin Core RPC {method} failed: {payload['error']}")
        return payload.get("result")

    def stop(self) -> None:
        command("docker", "stop", "--timeout", "3", self.name, check=False)

    def __enter__(self) -> BitcoinCoreRegtest:
        try:
            self.start()
        except BaseException:
            self.stop()
            raise
        return self

    def __exit__(self, *_: object) -> None:
        self.stop()


def build_signed_transaction(core: BitcoinCoreRegtest) -> tuple[str, str]:
    wallet = "epr"
    core.rpc("createwallet", wallet)
    mining_address = core.rpc("getnewaddress", wallet=wallet)
    core.rpc("generatetoaddress", 101, mining_address)
    destination = core.rpc("getnewaddress", wallet=wallet)
    raw = core.rpc("createrawtransaction", [], {destination: 1})
    funded = core.rpc("fundrawtransaction", raw, wallet=wallet)
    signed = core.rpc("signrawtransactionwithwallet", funded["hex"], wallet=wallet)
    if not signed.get("complete"):
        raise RuntimeError("Bitcoin Core did not completely sign the regtest transaction")
    decoded = core.rpc("decoderawtransaction", signed["hex"])
    return signed["hex"], decoded["txid"]


def run(binary: Path, image: str) -> None:
    with BitcoinCoreRegtest(image) as core:
        raw_transaction, transaction_id = build_signed_transaction(core)

        def relay_response(request: dict[str, Any]) -> dict[str, Any]:
            if request.get("method") != "blockchain.transaction.broadcast":
                raise RuntimeError("relay received a non-broadcast request")
            params = request.get("params")
            if params != [raw_transaction]:
                raise RuntimeError("relay received a different raw transaction")
            accepted_transaction_id = core.rpc("sendrawtransaction", raw_transaction)
            return {"id": request.get("id"), "result": accepted_transaction_id}

        with (
            ElectrumLineServer(standard_query_response) as query,
            ElectrumLineServer(relay_response) as relay,
            Socks5Forwarder(relay.port) as socks,
            RunningProxy(binary, query.port, relay.port, socks.port) as proxy,
            proxy.connect() as wallet,
        ):
            version = wallet.request(
                {
                    "id": 1,
                    "method": "server.version",
                    "params": ["regtest-e2e", "1.4"],
                }
            )
            assert version == {
                "id": 1,
                "result": ["electrum-private-relay-test", "1.4"],
            }

            broadcast = wallet.request(
                {
                    "id": 2,
                    "method": "blockchain.transaction.broadcast",
                    "params": [raw_transaction],
                }
            )
            assert broadcast == {"id": 2, "result": transaction_id}

            wait_for(lambda: len(relay.requests) == 1, "regtest relay request")
            assert "blockchain.transaction.broadcast" not in query.methods()
            assert socks.destinations == [("regtest-relay.invalid", relay.port)]
            mempool = core.rpc("getmempoolentry", transaction_id)
            assert mempool["vsize"] > 0

    print("regtest E2E passed")
    print(f"- Bitcoin Core image: {image}")
    print("- signed transaction accepted into a real regtest mempool")
    print("- broadcast isolated from the query upstream")
    print("- relay connection traversed the SOCKS5 adapter")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument(
        "--bitcoin-core-image",
        default=BITCOIN_CORE_IMAGE,
        help="Pinned test-only Bitcoin Core image reference",
    )
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not binary.is_file():
        raise SystemExit(f"proxy binary does not exist: {binary}")
    command("docker", "version")
    run(binary, args.bitcoin_core_image)


if __name__ == "__main__":
    main()
