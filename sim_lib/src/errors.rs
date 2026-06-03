use snafu::Snafu;

#[derive(Debug, Snafu)]
pub enum SimulationError {
    #[snafu(display("hardware error: {}", source))]
    #[snafu(context(false))]
    HardwareError { source: HardwareError },
    #[snafu(display("protocol error: {}", source))]
    #[snafu(context(false))]
    ProtocolError { source: ProtocolError },
}

#[derive(Debug, PartialEq, Snafu)]
pub enum HardwareError {
    GlobalCounterNotSynced,
    Timeout,
    ModulatorStateNotSupported,
    ResetFifoAtThisGcOverflow,
    FifoOverflow,
    Other { reason: String },
}

#[derive(Debug, PartialEq, Snafu)]
pub enum ProtocolError {
    Role { reason: String },
    ModulatorState { reason: String },
    Size { reason: String },
}
