use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("IPC error {}", source))]
    IpcReader {
        source: crate::ipc::reader::errors::Error,
    },
    #[snafu(display("Simulator error {}", source))]
    Backend {
        source: crate::backend::errors::Error,
    },
    #[snafu(display("UnixStream error {}", source))]
    UnixStream { source: tokio::io::Error },
    #[snafu(display("IO error {}", source))]
    IO { source: std::io::Error },
    #[snafu(display("SerdeJSON error {}", source))]
    SerdeJson { source: serde_json::Error },
}
