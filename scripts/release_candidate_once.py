#!/usr/bin/env python3
"""Build one release candidate and publish its evidence without overwrite.

The lower-level reproducible packager writes into a private temporary directory.
This wrapper is the supported local/CI entrypoint: it serializes publication per
target, verifies both final evidence paths are absent, and copies the ZIP and
SHA-256 sidecar with exclusive-create semantics. A failed publication removes
only files created by this invocation.
"""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import sys
import tempfile
from typing import Iterable

from reproducible_release import (
    PACKAGE_NAME,
    SUPPORTED_TARGETS,
    ReleaseCandidateError,
    build_release_candidate,
    package_version,
)


class CandidatePublicationLock:
    """Exclusive per-target lock stored inside the requested output directory."""

    def __init__(self, path: Path) -> None:
        self.path = path
        self._descriptor: int | None = None

    def __enter__(self) -> "CandidatePublicationLock":
        try:
            self._descriptor = os.open(
                self.path,
                os.O_CREAT | os.O_EXCL | os.O_WRONLY,
                0o600,
            )
        except FileExistsError as error:
            raise ReleaseCandidateError(
                f"candidate publication is already active: {self.path.name}"
            ) from error
        return self

    def __exit__(self, exc_type: object, exc: object, traceback: object) -> None:
        if self._descriptor is not None:
            os.close(self._descriptor)
            self._descriptor = None
        try:
            self.path.unlink()
        except FileNotFoundError:
            pass


def candidate_names(target: str, version: str) -> tuple[str, str]:
    """Return canonical archive and checksum filenames for one native target."""

    archive = f"{PACKAGE_NAME}-v{version}-{target}.zip"
    return archive, f"{archive}.sha256"


def require_absent(paths: Iterable[Path]) -> None:
    """Fail closed when any final evidence path already exists."""

    for path in paths:
        if path.exists():
            raise ReleaseCandidateError(
                f"refusing to overwrite existing candidate evidence: {path.name}"
            )


def copy_exclusive(source: Path, destination: Path) -> None:
    """Copy one file to a newly created destination without overwrite."""

    try:
        with source.open("rb") as reader, destination.open("xb") as writer:
            shutil.copyfileobj(reader, writer, length=1024 * 1024)
            writer.flush()
            os.fsync(writer.fileno())
    except FileExistsError as error:
        raise ReleaseCandidateError(
            f"refusing to overwrite existing candidate evidence: {destination.name}"
        ) from error


def publish_candidate_evidence(
    source_archive: Path,
    source_sidecar: Path,
    output_dir: Path,
) -> tuple[Path, Path]:
    """Publish an archive/sidecar pair with exclusive creation and rollback."""

    destination_archive = output_dir / source_archive.name
    destination_sidecar = output_dir / source_sidecar.name
    require_absent((destination_archive, destination_sidecar))

    created: list[Path] = []
    try:
        copy_exclusive(source_archive, destination_archive)
        created.append(destination_archive)
        copy_exclusive(source_sidecar, destination_sidecar)
        created.append(destination_sidecar)
    except Exception:
        for path in reversed(created):
            try:
                path.unlink()
            except FileNotFoundError:
                pass
        raise

    return destination_archive, destination_sidecar


def build_and_publish(root: Path, target: str, output_dir: Path) -> dict[str, object]:
    """Build in a private directory and publish exactly one immutable pair."""

    root = root.resolve()
    output_dir = output_dir.resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    if target not in SUPPORTED_TARGETS:
        raise ReleaseCandidateError(f"unsupported release-candidate target: {target}")

    version = package_version(root)
    archive_name, sidecar_name = candidate_names(target, version)
    final_archive = output_dir / archive_name
    final_sidecar = output_dir / sidecar_name
    lock_path = output_dir / f".{PACKAGE_NAME}-{target}.lock"

    with CandidatePublicationLock(lock_path):
        require_absent((final_archive, final_sidecar))
        with tempfile.TemporaryDirectory(
            prefix=f".{PACKAGE_NAME}-candidate-",
            dir=output_dir,
        ) as temporary_directory:
            temporary_output = Path(temporary_directory)
            summary = build_release_candidate(root, target, temporary_output)
            source_archive = temporary_output / str(summary["archive"])
            source_sidecar = temporary_output / str(summary["checksum_file"])
            if source_archive.name != archive_name or source_sidecar.name != sidecar_name:
                raise ReleaseCandidateError("lower-level packager returned unexpected filenames")
            publish_candidate_evidence(source_archive, source_sidecar, output_dir)

    return summary


def parse_arguments(arguments: Iterable[str] | None = None) -> argparse.Namespace:
    """Parse command-line arguments."""

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=SUPPORTED_TARGETS)
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
    )
    parser.add_argument("--output-dir", type=Path, default=Path("dist"))
    return parser.parse_args(arguments)


def main(arguments: Iterable[str] | None = None) -> int:
    """CLI entrypoint."""

    options = parse_arguments(arguments)
    try:
        build_and_publish(options.root, options.target, options.output_dir)
    except ReleaseCandidateError as error:
        print(f"release-candidate publication error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
