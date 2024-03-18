pub mod config;
pub mod reader;

use serde::Deserialize;
use serde::Serialize;

pub(crate) static NODE2HW: &str = "./node2hw.sock";

// These commands are specified by the Hardware team and are always between 0 and 255.
#[derive(Serialize, Deserialize, Debug)]
pub enum UsbCommand {
    Ok,       // = 0x16,           // reply in case there is nothing to reply
    FifoIdle, // = 0x26,     // set_modulator_state idle (stop writing to the fifo only)
    StartAtGc {
        gc: u64,
    }, // = 0x27,    // start modulating and writing to the fifo at gc
    ReadAngles, // = 0x28,   // read postselected (measured) angles
    GetCurrentGc, // = 0x29, // get current global counter
    AngleSet {
        angles: [u8; 8],
    }, // = 0x2a,     // set the angles, expected 8 values (byte) to follow
    KO,       // = 0xaa,           // This type does not exist in the real Hardware ?
    SetRole {
        number_of_parties: u32,
        position: u32,
    },
}

impl UsbCommand {
    fn as_bytes(&self) -> Vec<u8> {
        match self {
            UsbCommand::Ok => vec![0x16],
            UsbCommand::FifoIdle => vec![0x26],
            UsbCommand::StartAtGc { gc } => [vec![0x27], gc.to_be_bytes().to_vec()].concat(),
            UsbCommand::ReadAngles => vec![0x28],
            UsbCommand::GetCurrentGc => vec![0x29],
            UsbCommand::AngleSet { angles } => [vec![0x2a], angles.to_vec()].concat(),
            UsbCommand::KO => vec![0xaa],
            UsbCommand::SetRole {
                number_of_parties,
                position,
            } => [
                vec![0xab],
                number_of_parties.to_be_bytes().to_vec(),
                position.to_be_bytes().to_vec(),
            ]
            .concat(), // Should never be send by the simulator
        }
    }
}
