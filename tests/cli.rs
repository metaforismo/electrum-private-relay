// SPDX-License-Identifier: Apache-2.0

use std::process::Command;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_electrum-private-relay"))
}

#[test]
fn check_config_validates_without_opening_network_sockets() {
    let output = binary()
        .arg("--check-config")
        .output()
        .expect("binary should run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("configuration valid"));
    assert!(stdout.contains("no network connection was opened"));
}

#[test]
fn check_config_rejects_query_listener_loop() {
    let output = binary()
        .args(["--check-config", "--upstream", "127.0.0.1:50003"])
        .output()
        .expect("binary should run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("query upstream must not target the client listener"));
}

#[test]
fn check_config_rejects_query_and_relay_reuse() {
    let output = binary()
        .args([
            "--check-config",
            "--relay-mode",
            "socks-electrum",
            "--relay-endpoint",
            "localhost:50001",
        ])
        .output()
        .expect("binary should run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("private relay endpoint must differ from the query upstream"));
}

#[test]
fn check_config_rejects_zero_port_ip_endpoint() {
    let output = binary()
        .args(["--check-config", "--upstream", "127.0.0.1:0"])
        .output()
        .expect("binary should run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("endpoint port must be non-zero"));
}

#[test]
fn check_config_rejects_url_instead_of_host_port() {
    let output = binary()
        .args([
            "--check-config",
            "--upstream",
            "https://relay.example:50001",
        ])
        .output()
        .expect("binary should run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(
        "endpoint must use host:port without scheme, credentials, or path"
    ));
}
