#!/usr/bin/env python3
"""Tests for create-once release-candidate publication."""

from __future__ import annotations

from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from release_candidate_once import (
    CandidatePublicationLock,
    ReleaseCandidateError,
    candidate_names,
    publish_candidate_evidence,
    require_absent,
)


class ReleaseCandidateOnceTests(unittest.TestCase):
    def test_candidate_names_are_canonical(self) -> None:
        archive, sidecar = candidate_names("x86_64-unknown-linux-gnu", "0.1.0")
        self.assertEqual(
            archive,
            "electrum-private-relay-v0.1.0-x86_64-unknown-linux-gnu.zip",
        )
        self.assertEqual(sidecar, f"{archive}.sha256")

    def test_require_absent_rejects_existing_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "candidate.zip"
            sidecar = root / "candidate.zip.sha256"
            require_absent((archive, sidecar))

            archive.write_bytes(b"existing")
            with self.assertRaises(ReleaseCandidateError):
                require_absent((archive, sidecar))
            self.assertEqual(archive.read_bytes(), b"existing")

    def test_publication_creates_pair_without_overwrite(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            output = root / "output"
            source.mkdir()
            output.mkdir()
            archive = source / "candidate.zip"
            sidecar = source / "candidate.zip.sha256"
            archive.write_bytes(b"archive")
            sidecar.write_bytes(b"checksum")

            published_archive, published_sidecar = publish_candidate_evidence(
                archive,
                sidecar,
                output,
            )
            self.assertEqual(published_archive.read_bytes(), b"archive")
            self.assertEqual(published_sidecar.read_bytes(), b"checksum")

            with self.assertRaises(ReleaseCandidateError):
                publish_candidate_evidence(archive, sidecar, output)
            self.assertEqual(published_archive.read_bytes(), b"archive")
            self.assertEqual(published_sidecar.read_bytes(), b"checksum")

    def test_publication_does_not_replace_existing_sidecar(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            output = root / "output"
            source.mkdir()
            output.mkdir()
            archive = source / "candidate.zip"
            sidecar = source / "candidate.zip.sha256"
            archive.write_bytes(b"new-archive")
            sidecar.write_bytes(b"new-checksum")
            existing_sidecar = output / sidecar.name
            existing_sidecar.write_bytes(b"old-checksum")

            with self.assertRaises(ReleaseCandidateError):
                publish_candidate_evidence(archive, sidecar, output)
            self.assertFalse((output / archive.name).exists())
            self.assertEqual(existing_sidecar.read_bytes(), b"old-checksum")

    def test_partial_copy_is_removed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source"
            output = root / "output"
            source.mkdir()
            output.mkdir()
            archive = source / "candidate.zip"
            sidecar = source / "candidate.zip.sha256"
            archive.write_bytes(b"archive")
            sidecar.write_bytes(b"checksum")

            def fail_after_partial_copy(reader: object, writer: object, length: int) -> None:
                del reader, length
                writer.write(b"partial")
                raise OSError("synthetic copy failure")

            with patch(
                "release_candidate_once.shutil.copyfileobj",
                side_effect=fail_after_partial_copy,
            ):
                with self.assertRaises(OSError):
                    publish_candidate_evidence(archive, sidecar, output)

            self.assertFalse((output / archive.name).exists())
            self.assertFalse((output / sidecar.name).exists())

    def test_publication_lock_is_exclusive_and_removed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            lock_path = Path(directory) / ".candidate.lock"
            with CandidatePublicationLock(lock_path):
                self.assertTrue(lock_path.exists())
                with self.assertRaises(ReleaseCandidateError):
                    with CandidatePublicationLock(lock_path):
                        self.fail("second lock must not be acquired")
            self.assertFalse(lock_path.exists())


if __name__ == "__main__":
    unittest.main()
