pub mod reader;
pub mod writer;

use libhardware::ModulatorState;
use serde::Deserialize;
use serde::Serialize;

// These commands are specified by the Hardware team and are always between 0 and 255.
#[derive(Serialize, Deserialize, Debug)]
pub enum UsbCommand {
    Ok = 0x16,           // reply in case there is nothing to reply
    FifoIdle = 0x26,     // set_modulator_state idle (stop writing to the fifo only)
    StartAtGc = 0x27,    // start modulating and writing to the fifo at gc
    ReadAngles = 0x28,   // read postselected (measured) angles
    GetCurrentGc = 0x29, // get current global counter
    AngleSet = 0x2a,     // set the angles, expected 8 values (byte) to follow
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
