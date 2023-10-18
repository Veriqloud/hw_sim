use crate::backend::role::Role;
use crate::backend::simulation::errors::Error as SimulatorError;
use crate::backend::simulation::Simulator;
use rand::RngCore;

mod bb84_constants {
    // We work on batches of const size that are appended to v. It's faster that way.
    // The remainder in the last batch is returned as leftover.
    pub const BATCH: usize = 1 << 10;

    // masks for single bits
    pub const BASIS_BOB: u8 = 0b1;
    pub const STATE_BOB: u8 = 0b10;
    pub const BASIS_ALICE: u8 = 0b100;
    pub const STATE_ALICE: u8 = 0b1000;
    pub const RANDOM_BIT: u8 = 0b10000;
}

pub trait BB84 {
    fn correlations_bb84(&mut self, l: usize) -> Result<(Vec<u8>, Vec<u8>), SimulatorError>;
}

impl BB84 for Simulator {
    /// Simulate BB84 with qber.
    ///
    /// This function takes an already initialized RNG and generates a vector of bytes, where
    ///
    /// - bit 0 is the measurement result
    /// - bit 1 is the basis
    /// - bit 2 is the state
    ///
    /// The second returned vector contains leftovers that need to be fed into the function at the next
    /// call to keep synchronization in case of different sizes l between the calls done by Alice and Bob

    fn correlations_bb84(&mut self, l: usize) -> Result<(Vec<u8>, Vec<u8>), SimulatorError> {
        let mut v: Vec<u8> = Vec::with_capacity(l + bb84_constants::BATCH);
        v.extend(self.leftover.iter());
        let mut b = [0u8; bb84_constants::BATCH];

        // get another random array for the qber;
        let mut b_flip = [0u8; 2 * bb84_constants::BATCH];

        let threshold: u16 = (self.qb_err * (!0u16 as f64)) as u16;

        for _ in 0..l / bb84_constants::BATCH + 1 {
            self.rng.fill_bytes(&mut b);
            self.rng.fill_bytes(&mut b_flip);

            // flip tells us if to flip the result due to qb_err
            let mut flip: [bool; bb84_constants::BATCH] = [false; bb84_constants::BATCH];
            for (f, v) in flip
                .iter_mut()
                .zip(b_flip.chunks(2).map(|c| ((c[0] as u16) << 8) | c[1] as u16))
            {
                *f = v < threshold;
            }

            // do some work on the byte
            for (e, f) in b.iter_mut().zip(flip.iter()) {
                let mut result: u8;
                if (*e & bb84_constants::BASIS_ALICE) >> 2 == (*e & bb84_constants::BASIS_BOB) {
                    // if bases match
                    result = (*e & bb84_constants::STATE_ALICE) >> 3
                        ^ (*e & bb84_constants::STATE_BOB) >> 1; // xor the states
                    if *f {
                        result ^= 0b1
                    }; // if qber, flip result
                } else {
                    result = (*e & bb84_constants::RANDOM_BIT) >> 4;
                }

                // make the format the same for Alice and Bob
                match self.role {
                    Role::Sender => *e >>= 1,
                    Role::Receiver => *e <<= 1,
                    Role::OneOfMany(_) => {
                        // TODO
                    }
                }
                *e &= 0b110; // delete unused bits
                *e |= result;
            }
            v.extend(b.iter());
        }
        let new_leftover = v.split_off(l + self.lfifo_initial as usize);
        v.shrink_to_fit();
        Ok((v, new_leftover))
    }
}
