#!/usr/bin/env python3
"""Build and package a native electrum-private-relay candidate reproducibly.

The command performs two clean release builds for one Rust target, compares the
resulting executables byte-for-byte, runs deterministic packaged-binary checks
on both, and emits a deterministic ZIP archive plus a SHA-256 sidecar. It does
not publish a GitHub release, authenticate the source checkout, or prove
independent cross-machine reproducibility.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import tomllib
import zipfile
from typing import Any, Iterable, Mapping, Sequence

PACKAGE_NAME = "electrum-private-relay"
METADATA_SCHEMA = "electrum-private-relay/release-candidate/v1"
WINDOWS_MSVC_REPRO_FLAG = "-Clink-arg=/Brepro"
SUPPORTED_TARGETS = (
    "x86_64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
)
CONFIG_CHECK_OUTPUT = (
    "configuration valid; no listener was bound and no network connection was opened"
)
PACKAGE_DOCUMENTS = (
    "README.md",
    "SECURITY.md",
    "LICENSE",
    "docs/AUDIT_SCOPE.md",
    "docs/REPRODUCIBLE_BUILDS.md",
    "docs/architecture.md",
    "docs/relay-adapters.md",
    "docs/testing.md",
    "docs/tor.md",
)
ZIP_MIN_YEAR = 1980


class ReleaseCandidateError(RuntimeError):
    """Raised when a release-candidate invariant is not satisfied."""


def run_checked(
    command: Sequence[os.PathLike[str] | str],
    *,
    cwd: Path,
    env: Mapping[str, str] | None = None,
    capture_output: bool = False,
) -> subprocess.CompletedProcess[str]:
    """Run a subprocess with consistent diagnostics and strict failure handling."""

    printable = " ".join(os.fspath(part) for part in command)
    print(f"+ {printable}", file=sys.stderr)
    try:
        return subprocess.run(
            [os.fspath(part) for part in command],
            cwd=cwd,
            env=None if env is None else dict(env),
            check=True,
            text=True,
            capture_output=capture_output,
        )
    except FileNotFoundError as error:
        raise ReleaseCandidateError(f"required command not found: {command[0]}") from error
    except subprocess.CalledProcessError as error:
        detail = error.stderr.strip() if error.stderr else ""
        suffix = f": {detail}" if detail else ""
        raise ReleaseCandidateError(
            f"command failed with exit code {error.returncode}: {printable}{suffix}"
        ) from error


def command_output(command: Sequence[str], *, cwd: Path) -> str:
    """Return stripped stdout from a successful command."""

    return run_checked(command, cwd=cwd, capture_output=True).stdout.strip()


def sha256_bytes(data: bytes) -> str:
    """Return the lower-case SHA-256 digest of bytes."""

    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    """Return the lower-case SHA-256 digest of a file."""

    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def package_version(root: Path) -> str:
    """Read the package version from the root Cargo manifest."""

    with (root / "Cargo.toml").open("rb") as source:
        manifest = tomllib.load(source)
    try:
        name = manifest["package"]["name"]
        version = manifest["package"]["version"]
    except (KeyError, TypeError) as error:
        raise ReleaseCandidateError("Cargo.toml lacks package name/version") from error
    if name != PACKAGE_NAME or not isinstance(version, str):
        raise ReleaseCandidateError("Cargo.toml package identity is unexpected")
    return version


def source_date_epoch(root: Path) -> int:
    """Return the checked-out commit time as a stable build timestamp."""

    value = command_output(["git", "show", "-s", "--format=%ct", "HEAD"], cwd=root)
    try:
        epoch = int(value)
    except ValueError as error:
        raise ReleaseCandidateError("git returned an invalid commit timestamp") from error
    if epoch < 0:
        raise ReleaseCandidateError("commit timestamp must not be negative")
    return epoch


def validate_target(target: str) -> str:
    """Return a supported native target or fail closed."""

    if target not in SUPPORTED_TARGETS:
        supported = ", ".join(SUPPORTED_TARGETS)
        raise ReleaseCandidateError(
            f"unsupported release-candidate target {target!r}; expected one of: {supported}"
        )
    return target


def executable_name(target: str) -> str:
    """Return the platform-native executable name for a Rust target triple."""

    return f"{PACKAGE_NAME}.exe" if "windows" in target else PACKAGE_NAME


def is_windows_msvc_target(target: str) -> bool:
    """Return whether a Rust target uses the native Windows MSVC linker."""

    return target.endswith("-windows-msvc")


def reproducibility_rustflags(target: str, existing: str = "") -> str:
    """Append the target-specific linker flag used by the candidate gate."""

    parts = existing.split()
    if is_windows_msvc_target(target) and WINDOWS_MSVC_REPRO_FLAG not in parts:
        parts.append(WINDOWS_MSVC_REPRO_FLAG)
    return " ".join(parts)


def packaged_runtime_environment(environment: Mapping[str, str]) -> dict[str, str]:
    """Remove application configuration variables from packaged-binary checks."""

    return {key: value for key, value in environment.items() if not key.startswith("EPR_")}


def validate_clean_output(
    *,
    label: str,
    stdout: str,
    stderr: str,
    expected_stdout: str,
) -> str:
    """Validate one deterministic command result and return normalized stdout."""

    if stderr:
        raise ReleaseCandidateError(f"{label} unexpectedly wrote to stderr")
    normalized = stdout.strip()
    if normalized != expected_stdout:
        raise ReleaseCandidateError(
            f"{label} output mismatch: expected {expected_stdout!r}, got {normalized!r}"
        )
    return normalized


def build_binary(root: Path, target: str, target_dir: Path, epoch: int) -> Path:
    """Perform one clean locked release build and return the executable path."""

    environment = os.environ.copy()
    environment.update(
        {
            "CARGO_INCREMENTAL": "0",
            "CARGO_TARGET_DIR": os.fspath(target_dir),
            "SOURCE_DATE_EPOCH": str(epoch),
        }
    )
    rustflags = reproducibility_rustflags(target, environment.get("RUSTFLAGS", ""))
    if rustflags:
        environment["RUSTFLAGS"] = rustflags
    run_checked(
        ["cargo", "build", "--release", "--locked", "--target", target],
        cwd=root,
        env=environment,
    )
    binary = target_dir / target / "release" / executable_name(target)
    if not binary.is_file():
        raise ReleaseCandidateError(f"release binary was not created: {binary}")
    return binary


def run_packaged_checks(binary: Path, root: Path, version: str) -> dict[str, Any]:
    """Run deterministic no-network CLI checks against one packaged executable."""

    environment = packaged_runtime_environment(os.environ)
    version_result = run_checked(
        [binary, "--version"], cwd=root, env=environment, capture_output=True
    )
    version_output = validate_clean_output(
        label="version check",
        stdout=version_result.stdout,
        stderr=version_result.stderr,
        expected_stdout=f"{PACKAGE_NAME} {version}",
    )

    config_result = run_checked(
        [binary, "--check-config"], cwd=root, env=environment, capture_output=True
    )
    config_output = validate_clean_output(
        label="configuration check",
        stdout=config_result.stdout,
        stderr=config_result.stderr,
        expected_stdout=CONFIG_CHECK_OUTPUT,
    )
    return {
        "version_output": version_output,
        "check_config_output": config_output,
        "check_config_passed": True,
    }


def normalized_zip_datetime(epoch: int) -> tuple[int, int, int, int, int, int]:
    """Convert an epoch to the deterministic timestamp range supported by ZIP."""

    instant = dt.datetime.fromtimestamp(epoch, tz=dt.timezone.utc)
    if instant.year < ZIP_MIN_YEAR:
        instant = instant.replace(
            year=ZIP_MIN_YEAR,
            month=1,
            day=1,
            hour=0,
            minute=0,
            second=0,
        )
    second = instant.second - (instant.second % 2)
    return (instant.year, instant.month, instant.day, instant.hour, instant.minute, second)


def deterministic_json(value: Mapping[str, Any]) -> bytes:
    """Serialize JSON with stable key ordering and a final newline."""

    return (
        json.dumps(value, sort_keys=True, indent=2, ensure_ascii=True) + "\n"
    ).encode("utf-8")


def write_zip_entry(
    archive: zipfile.ZipFile,
    name: str,
    data: bytes,
    timestamp: tuple[int, int, int, int, int, int],
    mode: int,
) -> None:
    """Write one deterministic regular-file ZIP member."""

    info = zipfile.ZipInfo(filename=name, date_time=timestamp)
    info.create_system = 3
    info.compress_type = zipfile.ZIP_DEFLATED
    info.external_attr = (stat.S_IFREG | mode) << 16
    archive.writestr(info, data, compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)


def create_archive(
    *,
    root: Path,
    binary_data: bytes,
    binary_name: str,
    target: str,
    version: str,
    epoch: int,
    metadata: Mapping[str, Any],
    output_path: Path,
) -> None:
    """Create one deterministic target-specific ZIP release candidate."""

    prefix = f"{PACKAGE_NAME}-v{version}-{target}"
    timestamp = normalized_zip_datetime(epoch)
    members: list[tuple[str, bytes, int]] = [
        (f"{prefix}/{binary_name}", binary_data, 0o755),
        (f"{prefix}/BUILD-METADATA.json", deterministic_json(metadata), 0o644),
    ]
    for document in PACKAGE_DOCUMENTS:
        path = root / document
        if not path.is_file():
            raise ReleaseCandidateError(f"required package document is missing: {document}")
        members.append((f"{prefix}/{document}", path.read_bytes(), 0o644))

    output_path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output_path, mode="w") as archive:
        for name, data, mode in sorted(members, key=lambda member: member[0]):
            write_zip_entry(archive, name, data, timestamp, mode)


def write_checksum_sidecar(archive_path: Path) -> Path:
    """Write a conventional SHA-256 sidecar for one archive."""

    digest = sha256_file(archive_path)
    sidecar = archive_path.with_name(f"{archive_path.name}.sha256")
    sidecar.write_text(
        f"{digest}  {archive_path.name}\n",
        encoding="ascii",
        newline="\n",
    )
    return sidecar


def build_release_candidate(root: Path, target: str, output_dir: Path) -> dict[str, Any]:
    """Build twice, compare, check, package, and checksum one native target."""

    root = root.resolve()
    output_dir = output_dir.resolve()
    target = validate_target(target)
    version = package_version(root)
    epoch = source_date_epoch(root)
    commit = command_output(["git", "rev-parse", "HEAD"], cwd=root)
    rustc = command_output(["rustc", "--version", "--verbose"], cwd=root)
    cargo = command_output(["cargo", "--version", "--verbose"], cwd=root)

    with tempfile.TemporaryDirectory(
        prefix="electrum-private-relay-build-a-"
    ) as first_directory, tempfile.TemporaryDirectory(
        prefix="electrum-private-relay-build-b-"
    ) as second_directory:
        first_binary = build_binary(root, target, Path(first_directory), epoch)
        second_binary = build_binary(root, target, Path(second_directory), epoch)
        first_data = first_binary.read_bytes()
        second_data = second_binary.read_bytes()
        first_digest = sha256_bytes(first_data)
        second_digest = sha256_bytes(second_data)
        if first_data != second_data:
            raise ReleaseCandidateError(
                "same-runner clean release builds differ: "
                f"first={first_digest} second={second_digest}"
            )

        first_checks = run_packaged_checks(first_binary, root, version)
        second_checks = run_packaged_checks(second_binary, root, version)
        if first_checks != second_checks:
            raise ReleaseCandidateError(
                "the two release executables produced different packaged-check results"
            )

        metadata: dict[str, Any] = {
            "schema": METADATA_SCHEMA,
            "package": PACKAGE_NAME,
            "version": version,
            "commit": commit,
            "target": target,
            "source_date_epoch": epoch,
            "binary_sha256": first_digest,
            "same_runner_double_build": True,
            "rustc": rustc,
            "cargo": cargo,
            "linker_reproducibility_flags": (
                ["/Brepro"] if is_windows_msvc_target(target) else []
            ),
            "packaged_checks": first_checks,
        }
        archive_name = f"{PACKAGE_NAME}-v{version}-{target}.zip"
        archive_path = output_dir / archive_name
        create_archive(
            root=root,
            binary_data=first_data,
            binary_name=executable_name(target),
            target=target,
            version=version,
            epoch=epoch,
            metadata=metadata,
            output_path=archive_path,
        )
        sidecar = write_checksum_sidecar(archive_path)

    summary = {
        "schema": METADATA_SCHEMA,
        "target": target,
        "archive": archive_path.name,
        "archive_sha256": sha256_file(archive_path),
        "checksum_file": sidecar.name,
        "binary_sha256": first_digest,
        "same_runner_double_build": True,
        "packaged_checks_passed": True,
    }
    print(json.dumps(summary, sort_keys=True), flush=True)
    return summary


def parse_arguments(arguments: Iterable[str] | None = None) -> argparse.Namespace:
    """Parse command-line arguments."""

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--target",
        required=True,
        choices=SUPPORTED_TARGETS,
        help="supported native Rust target triple to build",
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parent.parent,
        help="repository root (default: parent of scripts/)",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=Path("dist"),
        help="directory for ZIP and checksum output",
    )
    return parser.parse_args(arguments)


def main(arguments: Iterable[str] | None = None) -> int:
    """CLI entrypoint."""

    options = parse_arguments(arguments)
    try:
        build_release_candidate(options.root, options.target, options.output_dir)
    except ReleaseCandidateError as error:
        print(f"release-candidate error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
