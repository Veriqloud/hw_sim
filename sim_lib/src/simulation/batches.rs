use bitvec::prelude::*;
use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};
use std::thread::JoinHandle;

use crate::{BATCH, BATCH_BYTES};

fn encode_gcr(gc: u64, result_bit: u8) -> [u8; 8] {
    let shifted_gc = gc >> 1;
    let gc_lsb = (gc & 1) as u8;
    let mut buffer = shifted_gc.to_le_bytes();
    buffer[6] = (buffer[6] & 0b1111_1100) | gc_lsb | ((result_bit & 1) << 1);
    buffer
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QkdBatch {
    /// Hardware GC value of the first event in this batch (gc_offset + pulses_before_batch).
    pub base_gc: u64,
    /// Pulse-counter step between consecutive detection events (≈ 1/η, rounded).
    /// Spacing between GC values within this batch in the GCR records.
    pub gc_step: u64,
    /// Index (0–3) into the angles table: encodes Alice's (basis, state) choice.
    #[serde(with = "serde_bytes")]
    pub alice_state_index: [u8; BATCH],
    /// Index (0–3) into the angles table: encodes Bob's (basis, state) choice.
    #[serde(with = "serde_bytes")]
    pub bob_state_index: [u8; BATCH],
    /// Measurement result for each event.
    pub results: BitArray<[u8; BATCH_BYTES], Lsb0>,
    /// true = decoy pulse (mu2), false = signal pulse (mu1). All false in non-decoy mode.
    pub decoy_states: BitArray<[u8; BATCH_BYTES], Lsb0>,
}

impl QkdBatch {
    /// Serialize Alice's data for the FIFO: 2 events per byte.
    /// Nibble layout: `0 [d][b1][b0]` — bits 3-0 of each nibble.
    pub fn to_alice_fifo(&self) -> Vec<u8> {
        (0..BATCH / 2)
            .map(|k| {
                let e0 = (self.alice_state_index[2 * k] & 0b11)
                    | ((self.decoy_states[2 * k] as u8) << 2);
                let e1 = (self.alice_state_index[2 * k + 1] & 0b11)
                    | ((self.decoy_states[2 * k + 1] as u8) << 2);
                e0 | (e1 << 4)
            })
            .collect()
    }

    /// Serialize Bob's angle data for the FIFO: 2 events per byte.
    /// Nibble layout: `00[b1][b0]`.
    pub fn to_bob_angle_fifo(&self) -> Vec<u8> {
        (0..BATCH / 2)
            .map(|k| {
                (self.bob_state_index[2 * k] & 0b11)
                    | ((self.bob_state_index[2 * k + 1] & 0b11) << 4)
            })
            .collect()
    }

    /// Encode all results as GCR records. GC values start from `self.base_gc`,
    /// stepping by `self.gc_step` per event (≈ 1/η pulses between detections).
    /// With `use_padding`, each record is followed by an 8-byte zero pad (hardware protocol).
    pub fn to_gcr_batch(&self, use_padding: bool) -> Vec<[u8; 8]> {
        let capacity = if use_padding { 2 * BATCH } else { BATCH };
        let mut out = Vec::with_capacity(capacity);
        for (i, result) in self.results.iter().enumerate() {
            out.push(encode_gcr(
                self.base_gc + i as u64 * self.gc_step,
                *result as u8,
            ));
            if use_padding {
                out.push([0u8; 8]);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use bitvec::prelude::*;

    use crate::{BATCH, BATCH_BYTES, simulation::batches::QkdBatch};

    #[test]
    fn alice_fifo_encodes_the_optional_intensity_bit_in_each_nibble() {
        let mut decoy_states = BitArray::<[u8; BATCH_BYTES], Lsb0>::ZERO;
        decoy_states.set(0, true);
        let mut alice_state_index = [0_u8; BATCH];
        alice_state_index[0] = 0b01;
        alice_state_index[1] = 0b10;
        let batch = QkdBatch {
            base_gc: 0,
            gc_step: 1,
            alice_state_index,
            bob_state_index: [0; BATCH],
            results: BitArray::ZERO,
            decoy_states,
        };

        assert_eq!(batch.to_alice_fifo()[0], 0b0010_0101);
    }

    #[test]
    fn alice_fifo_keeps_the_historical_format_without_decoy_states() {
        let mut alice_state_index = [0_u8; BATCH];
        alice_state_index[0] = 0b01;
        alice_state_index[1] = 0b10;
        let batch = QkdBatch {
            base_gc: 0,
            gc_step: 1,
            alice_state_index,
            bob_state_index: [0; BATCH],
            results: BitArray::ZERO,
            decoy_states: BitArray::ZERO,
        };

        assert_eq!(batch.to_alice_fifo()[0], 0b0010_0001);
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
