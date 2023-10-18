pub mod backend;
pub mod errors;
pub mod ipc;

use errors::{IOSnafu, IPCReaderSnafu, IpcWriterSnafu, UnixStreamSnafu};
use ipc::writer::{mock::MockInsert, Writer};
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
                let simu_handle = backend::actor::ActorHandle::new(backend::fake::MockSimu {});
                let mut ins = MockInsert {};
                ins.start().await.context(IpcWriterSnafu)?;
                let ins_handle = ipc::writer::actor::ActorHandle::new(ins);
                let ipc = ipc::reader::IPCReader::new(stream, simu_handle, ins_handle)
                    .await
                    .context(IPCReaderSnafu)?;
                ipc.start().await;
            }
            Err(e) => panic!("ERROR {e}"),
        }
    }
}
