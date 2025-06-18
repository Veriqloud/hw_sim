pub mod backend;
pub mod cli_args;
pub mod config;
pub mod errors;
pub mod ipc;

use crate::{
    backend::role::SimulatorMode,
    config::Configuration,
    ipc::config::{AliceIpcConfig, BobIpcConfig},
};
use backend::simulation::builder::SimulatorBuilder;
use clap::Parser;
use ipc::writer::actor::IPCWriterActorHandle;
use snafu::ResultExt;
use std::{sync::OnceLock, time::Duration};
use tokio::time::sleep;
use tracing::trace_span;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use uuid::Uuid;

pub static CONFIG: OnceLock<Configuration> = OnceLock::new();

#[tokio::main]
async fn main() {
    if let Err(e) = app_main().await {
        eprintln!("Application exited with error: {}", e);
        std::process::exit(1);
    }
}

// Core application logic
async fn app_main() -> Result<(), crate::errors::Error> {
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

    tracing::info!(
        "Simulator with configuration : {:?}",
        CONFIG.get().unwrap().backend_config
    );
    tracing::info!(
        "IPC with configuration : {:?}",
        CONFIG.get().unwrap().ipc_config
    );

    let simulator_mode = match CONFIG.get().unwrap().ipc_config {
        ipc::config::Configuration::Alice(_) => SimulatorMode::Source,
        ipc::config::Configuration::Bob(_) => SimulatorMode::Detector,
    };

    let sim = SimulatorBuilder::from_config(
        &CONFIG.get().unwrap().backend_config,
        simulator_mode.clone(),
    );
    let simu_handle = backend::actor::ActorHandle::new(sim);

    // The logic now diverges based on the IPC configuration type
    match &CONFIG.get().unwrap().ipc_config {
        ipc::config::Configuration::Alice(alice_config) => {
            run_alice_workflow(&alice_config, simu_handle, simulator_mode).await;
            tracing::error!("Alice's workflow function returned unexpectedly.");
        }
        ipc::config::Configuration::Bob(bob_config) => {
            run_bob_workflow(&bob_config, simu_handle, simulator_mode).await;
            tracing::error!("Bob's workflow function returned unexpectedly.");
        }
    }

    Ok(())
}

// Alice's (Source) workflow: waits for a controller connection in a loop.
async fn run_alice_workflow(
    config: &AliceIpcConfig,
    simu_handle: backend::actor::ActorHandle,
    simulator_mode: SimulatorMode,
) {
    loop {
        tracing::info!("Alice (Source) workflow: Waiting for a controller...");

        // For Alice, the GCR file is not used for writing data, but the IPCWriterActor
        // requires a file handle. We open /dev/null as a black hole for any potential writes.
        let gcr_file_writer = match tokio::fs::OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .await
        {
            Ok(file) => file,
            Err(e) => {
                tracing::error!("Failed to open /dev/null for GCR writer: {}. This is required for Alice's workflow. Exiting.", e);
                return;
            }
        };

        let angles_file_writer = match tokio::fs::OpenOptions::new()
            .write(true)
            .open(&config.angle_file_path)
            .await
        {
            Ok(file) => file,
            Err(e) => {
                tracing::error!(
                    "Failed to open angles_file_path '{}': {}. Retrying in 5s.",
                    &config.angle_file_path,
                    e
                );
                sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        let writer_handle = IPCWriterActorHandle::new(gcr_file_writer, angles_file_writer);

        let gc_read_file_handle = match tokio::fs::OpenOptions::new()
            .read(true)
            .open(&config.gc_read_file_path)
            .await
        {
            Ok(file) => file,
            Err(e) => {
                tracing::error!(
                    "Failed to open gc_read_file '{}': {}. Retrying in 5s.",
                    &config.gc_read_file_path,
                    e
                );
                sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        tracing::info!("IPC files opened. Initializing IPCReader for Alice.");
        let ipc_reader = ipc::reader::IPCReader::new(
            Some(config.command_path.clone()),
            gc_read_file_handle,
            simu_handle.clone(),
            writer_handle.clone(),
            simulator_mode.clone(),
        );

        tracing::info!("Starting IPC command processing loop for Alice.");
        if let Err(e) = ipc_reader.start().await {
            tracing::warn!(
                "IPC processing for Alice ended with an error: {:?}. Preparing for new connection.",
                e
            );
        } else {
            tracing::info!("IPCReader for Alice exited cleanly. Preparing for new connection.");
        }
        sleep(Duration::from_secs(5)).await;
    }
}

// Bob's (Detector) workflow: starts immediately, does not wait for controller.
async fn run_bob_workflow(
    config: &BobIpcConfig,
    simu_handle: backend::actor::ActorHandle,
    simulator_mode: SimulatorMode,
) {
    tracing::info!("Bob (Detector) workflow: Initializing IPC and starting generation.");

    let angles_file_writer = match tokio::fs::OpenOptions::new()
        .write(true)
        .open(&config.angle_file_path)
        .await
    {
        Ok(f) => f,
        Err(e) => {
            tracing::error!(
                "Bob workflow failed to open angles_file_path: {}. Exiting.",
                e
            );
            return;
        }
    };
    let gcr_file_writer = match tokio::fs::OpenOptions::new()
        .write(true)
        .open(&config.gcr_file_path)
        .await
    {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("Bob workflow failed to open gcr_file_path: {}. Exiting.", e);
            return;
        }
    };

    let writer_handle = IPCWriterActorHandle::new(gcr_file_writer, angles_file_writer);

    let gc_read_file_handle = match tokio::fs::OpenOptions::new()
        .read(true)
        .open(&config.gc_read_file_path)
        .await
    {
        Ok(f) => f,
        Err(e) => {
            tracing::error!(
                "Bob workflow failed to open gc_read_file_path: {}. Exiting.",
                e
            );
            return;
        }
    };

    tracing::info!("IPC files opened. Initializing IPCReader for Bob.");
    let ipc_reader = ipc::reader::IPCReader::new(
        Some(config.command_path.clone()), // Bob now uses a command path
        gc_read_file_handle,
        simu_handle,
        writer_handle,
        simulator_mode,
    );

    tracing::info!("Starting continuous generation loop for Bob.");
    if let Err(e) = ipc_reader.start().await {
        tracing::error!(
            "Bob's continuous generation loop exited with an error: {:?}",
            e
        );
    } else {
        tracing::warn!("Bob's IPC reader exited cleanly, which is unexpected.");
    }
}
