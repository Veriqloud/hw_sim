pub mod backend;
pub mod cli_args;
pub mod config;
pub mod errors;
pub mod ipc;

use backend::simulation::builder::SimulatorBuilder;
use clap::Parser;
use errors::IOSnafu;
use snafu::prelude::*;
use std::path::Path;
use tracing::trace_span;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use uuid::Uuid;

use crate::config::Configuration;

#[tokio::main]
async fn main() {
    //}-> Result<(), errors::Error> {
    let span = trace_span!("main");

    let args = cli_args::CliArgs::parse();

    let configuration: Configuration = if let Some(path) = args.conf.config_path {
        match Configuration::new(path) {
            Ok(c) => c,
            Err(e) => {
                span.in_scope(|| tracing::error!("{}", e));
                return;
            }
        }
    } else {
        let mut c = Configuration::default();

        if let Some(p) = args.conf.ipc_socket {
            c.ipc_config.unix_socket_path = p;
        }

        c
    };

    let config_string = serde_json::to_string(&configuration).unwrap();
    tracing::info!(
        "Running with configuration: {}",
        serde_json::to_string_pretty(&config_string).unwrap()
    );

    match TryInto::<tracing_subscriber::filter::LevelFilter>::try_into(configuration.log_level) {
        Ok(log_level) => {
            // Helps identify simulator sessions, and separate logs when multiples simulators are running on a single machine (local mode, for development)
            let log_id = Uuid::new_v4();
            let logfile = tracing_appender::rolling::daily(
                &args.logs_location,
                format!("simu_logs_{log_id}.log"),
            );
            let stdout = std::io::stdout.with_max_level(log_level.into_level().unwrap());
            tracing_subscriber::fmt()
                .with_writer(stdout.and(logfile))
                .init();
            tracing::info!("log_level: {:?}", log_level.into_level().unwrap());
            tracing::debug!("test DBG");
            tracing::info!(
                "log file name: {}",
                format!("{}/simu_logs_{}", args.logs_location, log_id)
            );
        }
        Err(e) => {
            println!("Could not initialize logger because {e}");
            return;
        }
    }

    tracing::info!(
        "Simulator with configuration : {:?}",
        &configuration.backend_config
    );

    let path = Path::new(&configuration.ipc_config.unix_socket_path);
    if path.exists() {
        std::fs::remove_file(path).context(IOSnafu).unwrap();
    }
    let listener =
        tokio::net::UnixListener::bind(configuration.ipc_config.unix_socket_path).unwrap();
    // .context(UnixStreamSnafu)?;
    tracing::info!("Listining to {:?}", listener.local_addr());

    let sim = SimulatorBuilder::from_config(configuration.backend_config);
    tracing::debug!("Simulator time: {:#?} ", sim.now);
    tracing::info!("Simulator modulator: {:?}", sim.role);
    let simu_handle = backend::actor::ActorHandle::new(sim);
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tracing::info!(
                    "Incoming stream from peer address {:?}",
                    &stream.peer_addr().unwrap()
                );
                let ipc = ipc::reader::IPCReader::new(stream, simu_handle.clone()).await;
                ipc.start().await;
                tracing::warn!("Socket died on client side.");
            }
            Err(e) => {
                tracing::error!("Error accepted the stream: {e}");
                return;
            }
        }
    }
}
