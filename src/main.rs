pub mod backend;
pub mod cli_args;
pub mod config;
pub mod errors;
pub mod ipc;

use backend::simulation::builder::SimulatorBuilder;
use clap::Parser;
use errors::UnixStreamSnafu;
use ipc::writer::actor::IPCWriterActorHandle;
use snafu::ResultExt;
use tokio::net::UnixListener;
use tracing::trace_span;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use uuid::Uuid;

use crate::config::Configuration;

#[tokio::main]
async fn main() {
    let span = trace_span!("main");

    let args = cli_args::CliArgs::parse();

    let configuration: Configuration = if let Some(path) = args.conf.config_path {
        match Configuration::new(path) {
            Ok(c) => c,
            Err(e) => {
                println!("ERROR: {}", e);
                span.in_scope(|| tracing::error!("{}", e));
                return;
            }
        }
    } else {
        Configuration::default()
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

    tracing::info!("IPC with configuration : {:?}", &configuration.ipc_config);

    configuration.ipc_config.check_field_exists().unwrap();

    let sim = SimulatorBuilder::from_config(configuration.backend_config);
    tracing::info!("Simulator modulator: {:?}", sim.role);
    let simu_handle = backend::actor::ActorHandle::new(sim);

    let angle_file = tokio::fs::OpenOptions::new()
        .truncate(true)
        .write(true)
        .open(configuration.ipc_config.angle_file_path)
        .await
        .unwrap();

    let click_result_file = tokio::fs::OpenOptions::new()
        .truncate(true)
        .write(true)
        .open(configuration.ipc_config.click_result_file_path)
        .await
        .unwrap();

    let writer_handle =
        IPCWriterActorHandle::new(angle_file, click_result_file, simu_handle.clone());

    let command_listener = UnixListener::bind(&configuration.ipc_config.command_socket_path)
        .context(UnixStreamSnafu)
        .unwrap();

    loop {
        let (command_stream, _) = command_listener
            .accept()
            .await
            .context(UnixStreamSnafu)
            .unwrap();

        tracing::info!(
            "Incoming stream from peer address {:?}",
            &command_stream.peer_addr().unwrap()
        );
        let ipc = ipc::reader::IPCReader::new(command_stream, writer_handle.clone()).await;
        if let Err(e) = ipc.start().await {
            tracing::error!("Error starting IPCReader: {:?}", e);
        }
    }
}
