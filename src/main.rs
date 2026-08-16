// SPDX-License-Identifier: Apache-2.0

#![forbid(unsafe_code)]

use std::{process::ExitCode, sync::Arc};

use clap::Parser;
use electrum_private_relay::{
    config::{Cli, Config, RelayMode},
    proxy::serve_until,
    relay::{RejectRelay, SharedRelay, SocksElectrumRelay},
};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fatal: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let check_config = cli.check_config;
    let config = Config::try_from(cli)?;
    if check_config {
        println!(
            "configuration valid; no listener was bound and no network connection was opened"
        );
        return Ok(());
    }

    let relay: SharedRelay = match config.relay_mode {
        RelayMode::Reject => Arc::new(RejectRelay),
        RelayMode::SocksElectrum => Arc::new(SocksElectrumRelay::new(
            config.socks5_proxy,
            config
                .relay_endpoint
                .clone()
                .expect("validated socks-electrum relay endpoint"),
            config.relay_timeout,
        )),
    };

    println!(
        "electrum-private-relay listening on {}; sensitive request logging is disabled",
        config.listen
    );
    serve_until(config, relay, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
