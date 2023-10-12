use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("IPC error {}", source))]
    IPCError { source: crate::ipc::errors::Error },
    #[snafu(display("Simulator error {}", source))]
    SimuError {
        source: crate::simulator::errors::Error,
    },
    #[snafu(display("UnixStream error {}", source))]
    UnixStreamError { source: tokio::io::Error },
}
