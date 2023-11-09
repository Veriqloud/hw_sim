pub mod backend;
pub mod errors;
pub mod ipc;

use backend::{role::Role, simulation::builder::SimulatorBuilder};
use errors::{IOSnafu, UnixStreamSnafu};
use ipc::writer::unix_stream::StreamWriter;
use libhardware::builder::HardwareBuilder;
use snafu::prelude::*;
use std::{path::Path, sync::Arc};
use tokio::io::BufWriter;

#[tokio::main]
async fn main() -> Result<(), errors::Error> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let path = Path::new("./node2hw");
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
    let stream = tokio::net::UnixStream::connect("./hw2node")
        .await
        .context(UnixStreamSnafu)?;
    let bw = BufWriter::new(stream);
    let w = Arc::new(bw);
    let _ins = StreamWriter { writer: w };
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let ipc = ipc::reader::IPCReader::new(stream, simu_handle.clone()).await;
                ipc.start().await;
            }
            Err(e) => panic!("ERROR {e}"),
        }
    }
}
