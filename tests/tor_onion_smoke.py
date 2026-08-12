#!/usr/bin/env python3
"""Exercise the SOCKS-Electrum adapter through a real ephemeral v3 onion."""

from __future__ import annotations

import argparse
import os
import secrets
import subprocess
import time
from pathlib import Path

from integration_harness import (
    ElectrumLineServer,
    RunningProxy,
    standard_query_response,
    successful_relay_response,
    unused_port,
    wait_for,
)


TOR_IMAGE = "electrum-private-relay/tor-smoke:0.4.9.11"
FAKE_TRANSACTION_ID = "22" * 32


def command(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        check=check,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


class TorPair:
    def __init__(self, relay_port: int) -> None:
        suffix = f"{os.getpid()}-{secrets.token_hex(4)}"
        self.service_name = f"epr-tor-service-{suffix}"
        self.client_name = f"epr-tor-client-{suffix}"
        self.socks_port = unused_port()
        self.relay_port = relay_port

    @staticmethod
    def build_image(repository: Path) -> None:
        command(
            "docker",
            "build",
            "--tag",
            TOR_IMAGE,
            str(repository / "tests" / "tor"),
        )

    def start(self) -> str:
        docker_operating_system = command(
            "docker", "info", "--format", "{{.OperatingSystem}}"
        ).stdout.strip()
        service_options: list[str]
        if docker_operating_system == "Docker Desktop":
            service_options = [
                "--add-host",
                "host.docker.internal:host-gateway",
            ]
        else:
            service_options = [
                "--network",
                "host",
                "--env",
                "EPR_TOR_RELAY_HOST=127.0.0.1",
            ]
        command(
            "docker",
            "run",
            "--detach",
            "--rm",
            "--name",
            self.service_name,
            *service_options,
            TOR_IMAGE,
            "service",
            str(self.relay_port),
        )
        command(
            "docker",
            "run",
            "--detach",
            "--rm",
            "--name",
            self.client_name,
            "--publish",
            f"127.0.0.1:{self.socks_port}:9050",
            TOR_IMAGE,
            "client",
        )

        self._wait_for_bootstrap(self.service_name)
        self._wait_for_bootstrap(self.client_name)
        hostname = command(
            "docker",
            "exec",
            self.service_name,
            "cat",
            "/var/lib/tor/onion/hostname",
        ).stdout.strip()
        if not hostname.endswith(".onion") or len(hostname) != 62:
            raise RuntimeError("Tor did not create a valid v3 onion hostname")
        return hostname

    def _wait_for_bootstrap(self, container_name: str) -> None:
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            logs = command("docker", "logs", container_name, check=False)
            combined = logs.stdout + logs.stderr
            if "Bootstrapped 100%" in combined:
                return
            state = command(
                "docker",
                "inspect",
                "--format",
                "{{.State.Running}}",
                container_name,
                check=False,
            )
            if state.returncode != 0 or state.stdout.strip() != "true":
                raise RuntimeError(f"Tor container exited before bootstrap:\n{combined}")
            time.sleep(0.25)
        logs = command("docker", "logs", container_name, check=False)
        raise RuntimeError(
            f"Tor did not bootstrap within 90 seconds:\n{logs.stdout}{logs.stderr}"
        )

    def stop(self) -> None:
        command(
            "docker",
            "stop",
            "--timeout",
            "3",
            self.client_name,
            self.service_name,
            check=False,
        )

    def __enter__(self) -> TorPair:
        return self

    def __exit__(self, *_: object) -> None:
        self.stop()


def run(binary: Path, repository: Path) -> None:
    TorPair.build_image(repository)
    with (
        ElectrumLineServer(standard_query_response) as query,
        ElectrumLineServer(successful_relay_response(FAKE_TRANSACTION_ID)) as relay,
        TorPair(relay.port) as tor,
    ):
        onion_hostname = tor.start()
        with (
            RunningProxy(
                binary,
                query.port,
                50_001,
                tor.socks_port,
                relay_host=onion_hostname,
                relay_timeout_seconds=45,
            ) as proxy,
            proxy.connect() as wallet,
        ):
            version = wallet.request(
                {
                    "id": 1,
                    "method": "server.version",
                    "params": ["tor-onion-smoke", "1.4"],
                }
            )
            assert version["id"] == 1
            broadcast = wallet.request(
                {
                    "id": 2,
                    "method": "blockchain.transaction.broadcast",
                    "params": ["00aa"],
                }
            )
            assert broadcast == {
                "id": 2,
                "result": FAKE_TRANSACTION_ID,
            }, f"unexpected broadcast response: {broadcast}"

        wait_for(lambda: len(relay.requests) == 1, "onion relay request")
        assert "blockchain.transaction.broadcast" not in query.methods()

    print("Tor v3 onion smoke passed")
    print("- client and onion-service Tor daemons bootstrapped to 100%")
    print("- SOCKS-Electrum reached the controlled relay through a v3 onion")
    print("- broadcast remained isolated from the query upstream")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    repository = Path(__file__).resolve().parent.parent
    command("docker", "version")
    run(binary, repository)


if __name__ == "__main__":
    main()
