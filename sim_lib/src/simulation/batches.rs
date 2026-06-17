use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};
use std::thread::JoinHandle;

use crate::BATCH;

/// Serde helper: serialize `[bool; BATCH]` as a compact byte array (one byte per bool).
mod serde_bool_array {
    use crate::BATCH;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(arr: &[bool; BATCH], ser: S) -> Result<S::Ok, S::Error> {
        let bytes: Vec<u8> = arr.iter().map(|&b| b as u8).collect();
        serde_bytes::serialize(bytes.as_slice(), ser)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<[bool; BATCH], D::Error> {
        let bytes: serde_bytes::ByteBuf = serde_bytes::deserialize(de)?;
        let mut arr = [false; BATCH];
        for (i, &b) in bytes.iter().enumerate().take(BATCH) {
            arr[i] = b != 0;
        }
        Ok(arr)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct QkdBatch {
    /// Index (0–3) into the angles table: encodes Alice's (basis, state) choice.
    #[serde(with = "serde_bytes")]
    pub alice_state_index: [u8; BATCH],
    /// Index (0–3) into the angles table: encodes Bob's (basis, state) choice.
    #[serde(with = "serde_bytes")]
    pub bob_state_index: [u8; BATCH],
    /// Measurement result for each event.
    #[serde(with = "serde_bool_array")]
    pub results: [bool; BATCH],
    /// true = decoy pulse (mu2), false = signal pulse (mu1). All false in non-decoy mode.
    #[serde(with = "serde_bool_array")]
    pub decoy_states: [bool; BATCH],
}

impl QkdBatch {
    /// Serialize Alice's data for the FIFO: 2 events per byte.
    /// Nibble layout: `0 [d][b1][b0]` — bits 3-0 of each nibble.
    pub fn to_alice_fifo(&self) -> Vec<u8> {
        (0..BATCH / 2)
            .map(|k| {
                let e0 = (self.alice_state_index[2 * k] & 0b11) | ((self.decoy_states[2 * k] as u8) << 2);
                let e1 = (self.alice_state_index[2 * k + 1] & 0b11) | ((self.decoy_states[2 * k + 1] as u8) << 2);
                e0 | (e1 << 4)
            })
            .collect()
    }

    /// Serialize Bob's angle data for the FIFO: 2 events per byte.
    /// Nibble layout: `00[b1][b0]`.
    pub fn to_bob_angle_fifo(&self) -> Vec<u8> {
        (0..BATCH / 2)
            .map(|k| {
                (self.bob_state_index[2 * k] & 0b11) | ((self.bob_state_index[2 * k + 1] & 0b11) << 4)
            })
            .collect()
    }

    /// Serialize Bob's click results for the FIFO: one byte per event (0 or 1).
    pub fn to_bob_click_fifo(&self) -> Vec<u8> {
        self.results.iter().map(|&r| r as u8).collect()
    }
}

/// Handle to manage a running QKD session.
pub struct SessionHandle {
    stop_tx: Sender<()>,
    thread_handle: Option<JoinHandle<()>>,
}

impl SessionHandle {
    /// Signals the session to stop and waits for the thread to exit.
    pub fn stop(mut self) {
        let _ = self.stop_tx.send(());
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }

    /// Sends a stop signal without waiting for the thread to exit.
    pub fn stop_async(&mut self) {
        let _ = self.stop_tx.send(());
    }
}
