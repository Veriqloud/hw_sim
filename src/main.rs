pub mod backend;
pub mod errors;
pub mod ipc;

use backend::{role::Role, simulation::builder::SimulatorBuilder};
use errors::{IOSnafu, IpcReaderSnafu, UnixStreamSnafu};
use ipc::writer::{mock::MockInsert, unix_stream::StreamWriter};
use libhardware::builder::HardwareBuilder;
use snafu::prelude::*;
use std::{path::Path, sync::Arc};
use tokio::io::BufWriter;

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
    let hw = HardwareBuilder::new().with_pulse_distance(1e-8).build();
    let sim = SimulatorBuilder::new()
        .with_role(Role::Sender)
        .with_eta(1e-2)
        .with_qb_err(0 as f64)
        .with_hardware(hw)
        .build();
    let simu_handle = backend::actor::ActorHandle::new(sim);
    let stream = tokio::net::UnixStream::connect("./output")
        // .context(IOSnafu)
        .await
        .unwrap();
    let bw = BufWriter::new(stream);
    let w = Arc::new(bw);
    let ins = StreamWriter { writer: w };
    let ins_handle = ipc::writer::actor::ActorHandle::new(ins);
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let ipc =
                    ipc::reader::IPCReader::new(stream, simu_handle.clone(), ins_handle.clone())
                        .await
                        .context(IpcReaderSnafu)?;
                ipc.start().await;
            }
            Err(e) => panic!("ERROR {e}"),
        }
    }
}
