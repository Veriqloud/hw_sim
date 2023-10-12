pub mod errors;
pub mod insertor;
pub mod ipc;
pub mod simulator;

use std::path::Path;

use insertor::Insertor;

#[tokio::main]
async fn main() -> Result<(), errors::Error> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let path = Path::new("./fifo_test");
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
    let listener = tokio::net::UnixListener::bind(path).unwrap();
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let simu_handle = simulator::actor::ActorHandle::new(simulator::fake::MockSimu {});
                let mut ins = insertor::fifo::MockInsert {};
                ins.start().await.unwrap();
                let ins_handle = insertor::actor::ActorHandle::new(ins);
                let ipc = ipc::IPCReader::new(stream, simu_handle, ins_handle)
                    .await
                    .unwrap();
                ipc.start().await;
            }
            Err(e) => panic!("ERROR {e}"),
        }
    }
}
