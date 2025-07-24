use snafu::Snafu;

#[derive(Debug, PartialEq, Snafu)]
pub enum Error {
    #[snafu(display("Protocol error : {}", source))]
    Protocol {
        source: crate::backend::protocols::errors::ProtocolError,
    },
}
