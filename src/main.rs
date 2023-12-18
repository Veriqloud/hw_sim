pub mod backend;
pub mod errors;
pub mod ipc;

use backend::{role::Role, simulation::builder::SimulatorBuilder};
use errors::{IOSnafu, UnixStreamSnafu};
use libhardware::builder::HardwareBuilder;
use snafu::prelude::*;
use std::{fs::OpenOptions, io::Read, path::Path, time::Instant};

use crate::{
    backend::{Angles, ANGLE_PATH},
    errors::SerdeJsonSnafu,
    ipc::NODE2HW,
};

#[tokio::main]
async fn main() -> Result<(), errors::Error> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let mut f = OpenOptions::new()
        .read(true)
        .open(ANGLE_PATH)
        .context(IOSnafu)?;
    let mut angles = String::new();
    f.read_to_string(&mut angles).context(IOSnafu)?;
    println!("ANGLES : {:?}", &angles);
    let angles: Angles = serde_json::from_str(&angles).context(SerdeJsonSnafu)?;

    let path = Path::new(NODE2HW);
    if path.exists() {
        std::fs::remove_file(path).context(IOSnafu)?;
    }
    let listener = tokio::net::UnixListener::bind(path).context(UnixStreamSnafu)?;
    let hw = HardwareBuilder::new().with_pulse_distance(1e-8).build();
    let sim = SimulatorBuilder::new()
        .with_role(Role::OneOfMany(backend::role::Multiparty {
            number_of_parties: 3,
            position: 0,
        }))
        .with_eta(1e-2)
        .with_qb_err(0 as f64)
        .with_hardware(hw)
        .with_angles(angles.angles.to_owned())
        .with_modulator_state(libhardware::ModulatorState::Random)
        .with_now(Instant::now())
        .build();
    tracing::debug!("Simulator time: {:#?} ", sim.now);
    tracing::debug!("Simulator modulator: {:?}", sim.role);
    let simu_handle = backend::actor::ActorHandle::new(sim);
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
