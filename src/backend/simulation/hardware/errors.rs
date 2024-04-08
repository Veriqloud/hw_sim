use snafu::Snafu;

#[derive(Debug, PartialEq, Snafu)]
pub enum HardwareError {
    GlobalCounterNotSynced,
    Timeout,
    ModulatorStateNotSupported,
    ResetFifoAtThisGcOverflow,
    FifoOverflow,
    Other { reason: String },
}
