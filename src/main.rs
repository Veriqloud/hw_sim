pub mod backend;
pub mod errors;
pub mod ipc;

use backend::{role::Role, simulation::builder::SimulatorBuilder};
use errors::{IOSnafu, IpcReaderSnafu, IpcWriterSnafu, UnixStreamSnafu};
use ipc::writer::{mock::MockInsert, Writer};
use libhardware::builder::HardwareBuilder;
use snafu::prelude::*;
use std::path::Path;

#[tokio::main]
async fn main() -> Result<(), errors::Error> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let path = Path::new("./fifo_test");
    if path.exists() {
        std::fs::remove_file(path).context(IOSnafu)?;
    }
    let listener = tokio::net::UnixListener::bind(path).context(UnixStreamSnafu)?;
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let hw = HardwareBuilder::new().with_pulse_distance(1e-8).build();
                let sim = SimulatorBuilder::new()
                    .with_role(Role::Sender)
                    .with_eta(1e-2)
                    .with_qb_err(0 as f64)
                    .with_hardware(hw)
                    .build();
                let simu_handle = backend::actor::ActorHandle::new(sim);
                let mut ins = MockInsert {};
                ins.start().await.context(IpcWriterSnafu)?;
                let ins_handle = ipc::writer::actor::ActorHandle::new(ins);
                let ipc = ipc::reader::IPCReader::new(stream, simu_handle, ins_handle)
                    .await
                    .context(IpcReaderSnafu)?;
                ipc.start().await;
            }
            Err(e) => panic!("ERROR {e}"),
        }
    }
}
