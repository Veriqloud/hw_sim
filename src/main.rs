pub mod backend;
pub mod cli_args;
pub mod config;
pub mod errors;
pub mod ipc;

use crate::config::Configuration;
use backend::simulation::builder::SimulatorBuilder;
use clap::Parser;
use errors::UnixStreamSnafu;
use ipc::writer::actor::IPCWriterActorHandle;
use nix;
use nix::sys::stat::Mode;
use snafu::ResultExt;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tokio::net::UnixListener;
use tokio::time::sleep; // Added for delays in the loop
use tracing::trace_span;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use uuid::Uuid;

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

    // Import the specific error type for matching IPC errors
    use crate::ipc::reader::errors::Error as IpcReaderError;

    loop {
        tracing::info!("Attempting to establish IPC connections. Waiting for a controller...");

        // Open angle_file (hw_sim: Write, controller: Read)
        // This will block until the controller opens its end for reading.
        let angle_file = match tokio::fs::OpenOptions::new()
            .write(true)
            .open(&configuration.ipc_config.angle_file_path)
            .await
        {
            Ok(file) => {
                tracing::info!(
                    "Opened angle_file: {}",
                    &configuration.ipc_config.angle_file_path
                );
                file
            }
            Err(e) => {
                tracing::error!(
                    "Failed to open angle_file '{}': {}. Retrying in 5s.",
                    &configuration.ipc_config.angle_file_path,
                    e
                );
                sleep(Duration::from_secs(5)).await;
                continue; // Retry the loop
            }
        };

        // Open gc_file (hw_sim: Read, controller: Write)
        let gc_file = match tokio::fs::OpenOptions::new()
            .read(true)
            .open(&configuration.ipc_config.gc_file_path)
            .await
        {
            Ok(file) => {
                tracing::info!("Opened gc_file: {}", &configuration.ipc_config.gc_file_path);
                file
            }
            Err(e) => {
                tracing::error!(
                    "Failed to open gc_file '{}': {}. Retrying in 5s.",
                    &configuration.ipc_config.gc_file_path,
                    e
                );
                sleep(Duration::from_secs(5)).await;
                continue; // Retry the loop
            }
        };

        // Open cmd_file (hw_sim: Read, controller: Write)
        let cmd_file = match tokio::fs::OpenOptions::new()
            .read(true)
            .open(&configuration.ipc_config.command_file_path)
            .await
        {
            Ok(file) => {
                tracing::info!(
                    "Opened cmd_file: {}",
                    &configuration.ipc_config.command_file_path
                );
                file
            }
            Err(e) => {
                tracing::error!(
                    "Failed to open cmd_file '{}': {}. Retrying in 5s.",
                    &configuration.ipc_config.command_file_path,
                    e
                );
                sleep(Duration::from_secs(5)).await;
                continue; // Retry the loop
            }
        };

        // Open click_result_file (hw_sim: Write, controller: Read)
        let click_result_file = match tokio::fs::OpenOptions::new()
            .write(true)
            .open(&configuration.ipc_config.click_result_file_path)
            .await
        {
            Ok(file) => {
                tracing::info!(
                    "Opened click_result_file: {}",
                    &configuration.ipc_config.click_result_file_path
                );
                file
            }
            Err(e) => {
                tracing::error!(
                    "Failed to open click_result_file '{}': {}. Retrying in 5s.",
                    &configuration.ipc_config.click_result_file_path,
                    e
                );
                sleep(Duration::from_secs(5)).await;
                continue; // Retry the loop
            }
        };

        tracing::info!("All IPC files opened successfully. Initializing IPC handlers.");
        let writer_handle =
            IPCWriterActorHandle::new(angle_file, click_result_file, simu_handle.clone());
        let ipc_reader =
            ipc::reader::IPCReader::new(cmd_file, gc_file, simu_handle.clone(), writer_handle);

        tracing::info!("IPC handlers initialized. Starting IPC command processing loop.");
        if let Err(e) = ipc_reader.start().await {
            match e {
                IpcReaderError::CommandFileIo { source }
                    if source.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    tracing::info!("Controller disconnected (EOF on command channel). Preparing for new connection.");
                }
                IpcReaderError::GcFileIo { source }
                    if source.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    tracing::info!("Controller disconnected (EOF on GC channel). Preparing for new connection.");
                }
                _ => {
                    tracing::warn!(
                        "IPC processing ended with an error: {:?}. Preparing for new connection.",
                        e
                    );
                }
            }
        } else {
            tracing::info!("IPCReader exited cleanly (this is unexpected if it's meant to run indefinitely). Preparing for new connection.");
        }

        tracing::info!(
            "Current IPC session ended. Will attempt to listen for a new controller connection."
        );
        // Files and handlers are dropped here as they go out of scope.
        // A small delay before restarting the loop to prevent tight looping on persistent errors.
        sleep(Duration::from_secs(1)).await;
    }
    // The main loop above will run indefinitely. The code below this point is effectively unreachable.
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
    tracing::info!("Creating FIFO at: {} with mode 0666", path_str);
    // Permissions: rw-rw-rw- (0o666)
    let mode = Mode::S_IRUSR
        | Mode::S_IWUSR
        | Mode::S_IRGRP
        | Mode::S_IWGRP
        | Mode::S_IROTH
        | Mode::S_IWOTH;
    nix::unistd::mkfifo(path, mode)?;
    Ok(())
}
