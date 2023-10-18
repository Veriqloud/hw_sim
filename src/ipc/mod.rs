pub mod reader;
pub mod writer;

use libhardware::ModulatorState;
use serde::Deserialize;
use serde::Serialize;

#[derive(Serialize, Deserialize, Debug)]
pub enum Message {
    ReadAnglesRequest,
    GetGlobalCounter,
    GetGcSafe,
    SetModulatorState,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ReadAnglesRequest {}

#[derive(Serialize, Deserialize, Debug)]
pub struct GetGlobalCounter {}

#[derive(Serialize, Deserialize, Debug)]
pub struct GetGcSafe {}

#[derive(Serialize, Deserialize, Debug)]
pub struct SetModulatorState {
    modulator_state: ModulatorState,
    at_global_counter: u64,
}
