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
use std::fs;
use std::os::unix::fs as unix_fs; // For mkfifo
use std::path::Path;
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

    // Ensure FIFOs are created/recreated before use
    tracing::info!("Ensuring IPC FIFOs are set up...");
    let ipc_paths = [
        &configuration.ipc_config.command_file_path,
        &configuration.ipc_config.angle_file_path,
        &configuration.ipc_config.click_result_file_path,
        &configuration.ipc_config.gc_file_path,
    ];

    for path_str in &ipc_paths {
        ensure_fifo(path_str).unwrap_or_else(|e| {
            tracing::error!("Failed to create FIFO at {}: {}. Exiting.", path_str, e);
            panic!("FIFO creation failed for {}: {}", path_str, e);
        });
    }
    tracing::info!("IPC FIFOs setup complete.");

    let sim = SimulatorBuilder::from_config(configuration.backend_config);
    tracing::info!("Simulator modulator: {:?}", sim.role);
    let simu_handle = backend::actor::ActorHandle::new(sim);

    let angle_file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(configuration.ipc_config.angle_file_path)
        .await
        .unwrap();

    let gc_file = tokio::fs::OpenOptions::new()
        .read(true)
        .open(configuration.ipc_config.gc_file_path)
        .await
        .unwrap();

    let cmd_file = tokio::fs::OpenOptions::new()
        .read(true)
        .open(configuration.ipc_config.command_file_path)
        .await
        .unwrap();

    let click_result_file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(configuration.ipc_config.click_result_file_path)
        .await
        .unwrap();

    let writer_handle =
        IPCWriterActorHandle::new(angle_file, click_result_file, simu_handle.clone());

    let ipc = ipc::reader::IPCReader::new(cmd_file, gc_file, simu_handle, writer_handle);
    ipc.start().await.unwrap();
    // let command_listener = UnixListener::bind(&configuration.ipc_config.command_file_path)
    //     .context(UnixStreamSnafu)
    //     .unwrap();

    // loop {
    //     let (command_stream, _) = command_listener
    //         .accept()
    //         .await
    //         .context(UnixStreamSnafu)
    //         .unwrap();

    //     tracing::info!(
    //         "Incoming stream from peer address {:?}",
    //         &command_stream.peer_addr().unwrap()
    //     );
    //     let ipc = ipc::reader::IPCReader::new(cmd_file, gc_file, simulator_handle, writer_handle);
    //     // let ipc = ipc::reader::IPCReader::new(command_stream, writer_handle.clone());
    //     if let Err(e) = ipc.start().await {
    //         tracing::error!("Error starting IPCReader: {:?}", e);
    //     }
    // }
}

/// Ensures that a FIFO exists at the given path.
/// If a file (or old FIFO) exists at the path, it is removed first.
/// Parent directories are created if they don't exist.
fn ensure_fifo(path_str: &str) -> Result<(), std::io::Error> {
    let path = Path::new(path_str);

    // Create parent directory if it doesn't exist
    if let Some(parent_dir) = path.parent() {
        if !parent_dir.exists() {
            fs::create_dir_all(parent_dir)?;
            tracing::info!("Created directory: {:?}", parent_dir);
        }
    }

    // Attempt to remove the file if it exists. This handles cases where it's a regular file
    // or an old FIFO that needs to be replaced.
    match fs::remove_file(path) {
        Ok(_) => tracing::info!("Removed existing file/FIFO at: {}", path_str),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // File not found, which is fine, we'll create it.
            tracing::debug!(
                "No existing file/FIFO at: {}. Proceeding to create.",
                path_str
            );
        }
        Err(e) => {
            // For other errors during removal, log and return the error.
            tracing::error!("Error removing existing file/FIFO at {}: {}", path_str, e);
            return Err(e);
        }
    }

    // Create the FIFO.
    tracing::info!("Creating FIFO at: {}", path_str);
    unix_fs::mkfifo(path, 0o666)?; // Permissions: rw-rw-rw-
    Ok(())
}
