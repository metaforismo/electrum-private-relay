#!/usr/bin/env python3
"""Unit tests for deterministic electrum-private-relay candidate packaging."""

from __future__ import annotations

import json
import os
from pathlib import Path
import tempfile
import unittest
import zipfile

from reproducible_release import (
    CONFIG_CHECK_OUTPUT,
    METADATA_SCHEMA,
    PACKAGE_DOCUMENTS,
    ReleaseCandidateError,
    SUPPORTED_TARGETS,
    create_archive,
    normalized_zip_datetime,
    packaged_runtime_environment,
    reproducibility_rustflags,
    sha256_file,
    validate_clean_output,
    validate_target,
    write_checksum_sidecar,
)


class ReproducibleReleaseTests(unittest.TestCase):
    def test_zip_timestamp_is_utc_and_rounded_to_two_seconds(self) -> None:
        self.assertEqual(
            normalized_zip_datetime(1_704_165_845),
            (2024, 1, 2, 3, 24, 4),
        )

    def test_windows_reproducibility_flag_is_targeted_and_idempotent(self) -> None:
        existing = "-Dwarnings"
        expected = "-Dwarnings -Clink-arg=/Brepro"
        self.assertEqual(
            reproducibility_rustflags("x86_64-pc-windows-msvc", existing),
            expected,
        )
        self.assertEqual(
            reproducibility_rustflags("x86_64-pc-windows-msvc", expected),
            expected,
        )
        self.assertEqual(
            reproducibility_rustflags("x86_64-unknown-linux-gnu", existing),
            existing,
        )

    def test_release_target_allowlist_fails_closed(self) -> None:
        for target in SUPPORTED_TARGETS:
            self.assertEqual(validate_target(target), target)
        for target in ("../../escape", "wasm32-wasi", "x86_64-unknown-linux-musl"):
            with self.assertRaises(ReleaseCandidateError):
                validate_target(target)

    def test_packaged_runtime_environment_removes_epr_configuration(self) -> None:
        environment = {
            "PATH": os.environ.get("PATH", ""),
            "EPR_LISTEN": "0.0.0.0:1",
            "EPR_RELAY_MODE": "socks-electrum",
        }
        cleaned = packaged_runtime_environment(environment)
        self.assertIn("PATH", cleaned)
        self.assertNotIn("EPR_LISTEN", cleaned)
        self.assertNotIn("EPR_RELAY_MODE", cleaned)

    def test_clean_output_validation_is_exact(self) -> None:
        self.assertEqual(
            validate_clean_output(
                label="configuration check",
                stdout=f"{CONFIG_CHECK_OUTPUT}\n",
                stderr="",
                expected_stdout=CONFIG_CHECK_OUTPUT,
            ),
            CONFIG_CHECK_OUTPUT,
        )
        with self.assertRaises(ReleaseCandidateError):
            validate_clean_output(
                label="configuration check",
                stdout="unexpected\n",
                stderr="",
                expected_stdout=CONFIG_CHECK_OUTPUT,
            )
        with self.assertRaises(ReleaseCandidateError):
            validate_clean_output(
                label="configuration check",
                stdout=f"{CONFIG_CHECK_OUTPUT}\n",
                stderr="warning\n",
                expected_stdout=CONFIG_CHECK_OUTPUT,
            )

    def test_archive_is_byte_for_byte_deterministic(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for document in PACKAGE_DOCUMENTS:
                path = root / document
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(f"{document}\n", encoding="utf-8", newline="\n")

            metadata = {
                "schema": METADATA_SCHEMA,
                "commit": "11" * 20,
                "target": "x86_64-unknown-linux-gnu",
                "same_runner_double_build": True,
                "packaged_checks": {"check_config_passed": True},
            }
            first = root / "first.zip"
            second = root / "second.zip"
            arguments = {
                "root": root,
                "binary_data": b"synthetic-public-test-binary\n",
                "binary_name": "electrum-private-relay",
                "target": "x86_64-unknown-linux-gnu",
                "version": "0.1.0",
                "epoch": 1_704_165_845,
                "metadata": metadata,
            }
            create_archive(output_path=first, **arguments)
            create_archive(output_path=second, **arguments)
            self.assertEqual(first.read_bytes(), second.read_bytes())

            with zipfile.ZipFile(first) as archive:
                names = archive.namelist()
                self.assertEqual(names, sorted(names))
                metadata_name = next(
                    name for name in names if name.endswith("BUILD-METADATA.json")
                )
                parsed = json.loads(archive.read(metadata_name))
                self.assertEqual(parsed["schema"], METADATA_SCHEMA)
                executable = next(
                    name for name in names if name.endswith("/electrum-private-relay")
                )
                mode = archive.getinfo(executable).external_attr >> 16
                self.assertEqual(mode & 0o777, 0o755)
                for document in PACKAGE_DOCUMENTS:
                    self.assertTrue(any(name.endswith(f"/{document}") for name in names))

    def test_checksum_sidecar_matches_archive(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "candidate.zip"
            archive.write_bytes(b"candidate")
            sidecar = write_checksum_sidecar(archive)
            expected = f"{sha256_file(archive)}  {archive.name}\n"
            self.assertEqual(sidecar.read_text(encoding="ascii"), expected)


if __name__ == "__main__":
    unittest.main()
