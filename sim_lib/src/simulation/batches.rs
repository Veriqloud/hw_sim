use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};
use std::thread::JoinHandle;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct QkdBatch {
    #[serde(with = "serde_bytes")]
    pub click_results: [u8; 1024],
    #[serde(with = "serde_bytes")]
    pub alice_angles: [u8; 1024],
    #[serde(with = "serde_bytes")]
    pub bob_angles: [u8; 1024],
    /// Decoy state label per event: 0 = signal (µ1), 1 = decoy (µ2).
    /// All zeros in non-decoy mode.
    #[serde(with = "serde_bytes")]
    pub decoy_states: [u8; 1024],
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
