// SPDX-License-Identifier: Apache-2.0

#![forbid(unsafe_code)]

use std::{process::ExitCode, sync::Arc, time::Duration};

use clap::Parser;
use electrum_private_relay::{
    config::{Cli, Config, RelayMode},
    proxy::serve_until,
    relay::{DrainingRelay, LimitedRelay, RejectRelay, SharedRelay, SocksElectrumRelay},
};
use tokio::sync::oneshot;

const SHUTDOWN_RESPONSE_FLUSH_GRACE: Duration = Duration::from_secs(1);

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
        println!("configuration valid; no listener was bound and no network connection was opened");
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
    let relay: SharedRelay = Arc::new(LimitedRelay::new(relay, config.max_concurrent_broadcasts));
    let (relay, drain) = DrainingRelay::new(relay);
    let relay: SharedRelay = Arc::new(relay);
    let maximum_drain = config.relay_timeout;

    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let signal_drain = drain.clone();
    let signal_task = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = signal_drain.begin_shutdown();
        let _ = shutdown_sender.send(());
    });

    println!(
        "electrum-private-relay listening on {}; sensitive request logging is disabled",
        config.listen
    );
    let server_result = serve_until(config, relay, async {
        let _ = shutdown_receiver.await;
    })
    .await;

    if let Err(error) = server_result {
        signal_task.abort();
        return Err(Box::new(error));
    }
    let _ = signal_task.await;

    if drain.wait_for_idle(maximum_drain).await {
        tokio::time::sleep(SHUTDOWN_RESPONSE_FLUSH_GRACE).await;
    } else {
        eprintln!(
            "shutdown relay drain expired; remaining private relay work will be cancelled without fallback"
        );
    }
    Ok(())
}
