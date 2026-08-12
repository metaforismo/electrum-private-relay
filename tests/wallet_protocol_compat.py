#!/usr/bin/env python3
"""Black-box protocol profiles derived from current wallet source trees."""

from __future__ import annotations

import argparse
from pathlib import Path

from integration_harness import (
    ElectrumLineServer,
    RunningProxy,
    Socks5Forwarder,
    standard_query_response,
    successful_relay_response,
    wait_for,
)


FAKE_TRANSACTION_ID = "11" * 32
FAKE_RAW_TRANSACTION = "00aa"

# Source snapshots inspected when these wire profiles were added.
SOURCE_SNAPSHOTS = {
    "Electrum": "spesmilo/electrum@c4cc40fdb8555b21f45da91ea7f85f1145907aea",
    "Sparrow": "sparrowwallet/sparrow@b99b880c9fe75565921af9ef438d6314fdd73d6f",
    "BlueWallet": "BlueWallet/BlueWallet@d8cb05c3997d3ead42487cc79008d3f59b539707",
}

PROFILES = [
    {
        "name": "Electrum",
        "version_params": ["Electrum compatibility harness", ["1.4", "1.4.2"]],
        "follow_up": "server.features",
    },
    {
        "name": "Sparrow",
        "version_params": ["Sparrow compatibility harness", "1.4"],
        "follow_up": "server.ping",
    },
    {
        "name": "BlueWallet",
        "version_params": ["bluewallet", "1.4"],
        "follow_up": "blockchain.headers.subscribe",
    },
]


def run(binary: Path) -> None:
    with (
        ElectrumLineServer(standard_query_response) as query,
        ElectrumLineServer(successful_relay_response(FAKE_TRANSACTION_ID)) as relay,
        Socks5Forwarder(relay.port) as socks,
        RunningProxy(binary, query.port, relay.port, socks.port) as proxy,
    ):
        for profile_number, profile in enumerate(PROFILES, start=1):
            with proxy.connect() as wallet:
                version_id = profile_number * 10
                version = wallet.request(
                    {
                        "id": version_id,
                        "method": "server.version",
                        "params": profile["version_params"],
                    }
                )
                assert version == {
                    "id": version_id,
                    "result": ["electrum-private-relay-test", "1.4"],
                }

                follow_up_id = version_id + 1
                follow_up = wallet.request(
                    {
                        "id": follow_up_id,
                        "method": profile["follow_up"],
                        "params": [],
                    }
                )
                assert follow_up.get("id") == follow_up_id
                assert "result" in follow_up

                broadcast_id = version_id + 2
                broadcast = wallet.request(
                    {
                        "id": broadcast_id,
                        "method": "blockchain.transaction.broadcast",
                        "params": [FAKE_RAW_TRANSACTION],
                    }
                )
                assert broadcast == {"id": broadcast_id, "result": FAKE_TRANSACTION_ID}

        wait_for(
            lambda: len(relay.requests) == len(PROFILES),
            "all wallet-profile broadcasts",
        )
        assert "blockchain.transaction.broadcast" not in query.methods()
        assert relay.methods() == ["blockchain.transaction.broadcast"] * len(PROFILES)
        assert socks.destinations == [
            ("regtest-relay.invalid", relay.port)
        ] * len(PROFILES)

    print("wallet protocol profiles passed:")
    for profile in PROFILES:
        print(f"- {profile['name']}: {SOURCE_SNAPSHOTS[profile['name']]}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not binary.is_file():
        raise SystemExit(f"proxy binary does not exist: {binary}")
    run(binary)


if __name__ == "__main__":
    main()
