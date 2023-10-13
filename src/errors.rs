use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("IPC error {}", source))]
    IPC { source: crate::ipc::errors::Error },
    #[snafu(display("Simulator error {}", source))]
    Simu {
        source: crate::simulator::errors::Error,
    },
    #[snafu(display("Insertor error {}", source))]
    IpcWriter {
        source: crate::ipc::writer::errors::Error,
    },
    #[snafu(display("UnixStream error {}", source))]
    UnixStream { source: tokio::io::Error },
    #[snafu(display("IO error {}", source))]
    IO { source: std::io::Error },
}
