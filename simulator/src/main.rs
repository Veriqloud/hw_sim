pub mod backend;
pub mod cli_args;
pub mod errors;
pub mod hardware_session;
pub mod ipc;
pub mod runtime_control;
pub mod runtime_status;

use clap::Parser;
use configs::Configuration;
use hardware_session::supervisor::HardwareSessionSupervisor;
use runtime_control::start_runtime_control_server;
use runtime_status::RuntimeStatusFiles;
use sim_lib::{hardware::modes::SimulatorMode, simulation::builder::SimulatorBuilder};
use snafu::ResultExt;
use std::{
    io::{Seek, SeekFrom, Write},
    sync::OnceLock,
};
use tracing::trace_span;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use uuid::Uuid;

pub static CONFIG: OnceLock<Configuration> = OnceLock::new();

fn main() {
    if let Err(e) = app_main() {
        eprintln!("Application exited with error: {}", e);
        std::process::exit(1);
    }
}

// Core application logic
fn app_main() -> Result<(), crate::errors::Error> {
    let span = trace_span!("app_main");
    let _guard = span.enter();

    let args = cli_args::CliArgs::parse();

    let configuration: Configuration = if let Some(path) = args.conf.config_path {
        Configuration::new(path).context(errors::ConfigLoadSnafu)?
    } else {
        Configuration::default()
    };

    CONFIG
        .set(configuration)
        .expect("failed to set the config global var\n");

    tracing::info!(
        "Running with configuration: {}",
        serde_json::to_string_pretty(&CONFIG.get().unwrap())
            .unwrap_or_else(|e| format!("Failed to serialize config for logging: {}", e))
    );

    // Initialize logger
    let log_level_filter = TryInto::<tracing_subscriber::filter::LevelFilter>::try_into(
        CONFIG.get().unwrap().log_level.to_owned(),
    )
    .context(errors::LoggerInitializationSnafu)?;

    let log_id = Uuid::new_v4();
    let logfile_name = format!("simu_logs_{log_id}.log");
    let logfile_path_str = format!("{}/{}", args.logs_location, &logfile_name);

    let logfile_appender = tracing_appender::rolling::daily(&args.logs_location, &logfile_name);
    let stdout_level = log_level_filter
        .into_level()
        .unwrap_or(tracing::Level::INFO);
    let stdout_writer = std::io::stdout.with_max_level(stdout_level);

    tracing_subscriber::fmt()
        .with_max_level(log_level_filter)
        .with_writer(stdout_writer.and(logfile_appender))
        .init();

    tracing::info!("log_level: {:?}", stdout_level);
    tracing::info!("log file path: {}", logfile_path_str);

    // Setup IPC FIFOs using the method on ipc_config
    CONFIG
        .get()
        .unwrap()
        .ipc_config
        .setup_ipc_fifos()
        .context(errors::IpcConfigSnafu)?;

    let runtime_status = RuntimeStatusFiles::new(
        CONFIG.get().unwrap().ipc_config.qkd_ready_path(),
        CONFIG.get().unwrap().ipc_config.node_idle_path(),
    );
    runtime_status.initialize().context(errors::IOSnafu)?;

    tracing::info!(
        "Simulator with configuration : {:?}",
        CONFIG.get().unwrap().backend_config
    );
    tracing::info!(
        "IPC with configuration : {:?}",
        CONFIG.get().unwrap().ipc_config
    );

    let simulator_mode = match CONFIG.get().unwrap().ipc_config {
        configs::ipc::Configuration::Alice(_) => SimulatorMode::Source,
        configs::ipc::Configuration::Bob(_) => SimulatorMode::Detector,
    };

    let sim = SimulatorBuilder::from_config(&CONFIG.get().unwrap().backend_config, simulator_mode);
    let simu_handle = backend::actor::ActorHandle::new(sim);
    let runtime_control = start_runtime_control_server(
        CONFIG.get().unwrap().ipc_config.control_socket_path(),
        simu_handle.clone(),
    )
    .context(errors::IOSnafu)?;

    let ipc_config = &CONFIG.get().unwrap().ipc_config;
    match ipc_config {
        configs::ipc::Configuration::Alice(alice_config) => {
            tracing::info!("Attempting to trigger initial PPS for Alice...");
            if let Err(e) = trigger_pps(&alice_config.command_path) {
                tracing::error!(
                    "Failed to trigger initial PPS for Alice: {}. Continuing...",
                    e
                );
            } else {
                tracing::info!("Initial PPS for Alice triggered successfully.");
            }
        }
        configs::ipc::Configuration::Bob(bob_config) => {
            tracing::info!("Attempting to trigger initial PPS for Bob...");
            if let Err(e) = trigger_pps(&bob_config.command_path) {
                tracing::error!(
                    "Failed to trigger initial PPS for Bob: {}. Continuing...",
                    e
                );
            } else {
                tracing::info!("Initial PPS for Bob triggered successfully.");
            }
        }
    }

    HardwareSessionSupervisor::new(ipc_config, simu_handle, runtime_control, runtime_status).run();
    tracing::error!("Hardware session supervisor returned unexpectedly.");

    Ok(())
}

/// Writes a '1' to a specific offset in the command file to trigger a PPS signal.
fn trigger_pps(command_path: &str) -> Result<(), std::io::Error> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(command_path)?;

    // As per the sample function, write to offset 0x1000 + 48.
    let absolute_offset = 0x1000u64 + 48u64;

    // Seek to the position and write the value.
    file.seek(SeekFrom::Start(absolute_offset))?;
    file.write_all(&1u32.to_le_bytes())?;
    file.flush()?;

    tracing::info!(
        "Successfully wrote 1u32 to offset 0x{:X} in file {} for PPS trigger.",
        absolute_offset,
        command_path
    );
    Ok(())
}
