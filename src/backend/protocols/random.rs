use crate::backend::role::Role;
use crate::backend::simulation::errors::Error as SimulatorError;
use crate::backend::simulation::Simulator;
use libhardware::ModulatorState;
//use itertools::izip;
use rand::RngCore;
use std::f32::consts::PI;

mod cr_constants {
    // We work on batches of const size that are appended to v. It's faster that way.
    // The remainder in the last batch is returned as leftover.
    pub const BATCH: usize = 1 << 10;
}

pub trait CorrelationsRandom {
    fn correlations_random(&mut self, l: usize) -> Result<(Vec<u8>, Vec<u8>), SimulatorError>;
}

impl CorrelationsRandom for Simulator {
    /// Simulate any angles.
    ///
    /// Encoding for returned bytes:
    ///
    /// - bit 0 is the measurement result (all parties have this result, not just Bob as in the real world)
    /// - bit 1 to 7 is the angle where 0=128 corresponds to 2pi with result bit 0 and 64 to pi
    /// with result bit 1
    ///
    /// If the quber is not zero, the result bit will flip sometimes
    ///
    /// The second returned vector contains leftovers that need to be fed into the function at the next
    /// call to keep synchronization in case of different sizes l between the calls done by Alice and Bob

    fn correlations_random(&mut self, l: usize) -> Result<(Vec<u8>, Vec<u8>), SimulatorError> {
        // number of players n and my id k
        let n;
        let k;
        match &self.role {
            Role::OneOfMany(m) => {
                n = m.number_of_parties;
                k = m.position;
            }
            _ => panic!(
                "Role not supported. Only Role::OneOfMany is supported for correlations_random"
            ),
        }

        // the output vector
        let mut v: Vec<u8> = Vec::with_capacity(l + cr_constants::BATCH);
        v.extend(self.leftover.iter());

        // number of parties n, my positon k
        let mut b_parties = Vec::new();
        //let mut b1 = [0u8; 2 * cr_constants::BATCH];
        //let mut b2 = [0u8; 2 * cr_constants::BATCH];
        // one random array to draw the result
        let mut br = [0u8; 2 * cr_constants::BATCH];
        // another random array for the qber
        let mut b_flip = [0u8; 2 * cr_constants::BATCH];

        // translate qber to u16
        let threshold: u16 = (self.qb_err * (!0u16 as f64)) as u16;

        // we are going to copy the angles into a fixed size array
        let mut angles = [0u8; 128];
        let num_angles: u16;
        match &self.modulator_state {
            ModulatorState::Random(angles_vec) => {
                num_angles = angles_vec.len() as u16;
                for (a1, a2) in angles.iter_mut().zip(angles_vec) {
                    *a1 = *a2;
                }
            }
            _ => panic!("modulator state in correlations_random is not Random"),
        }

        // get overlaps
        let o = overlaps();

        for _ in 0..l / cr_constants::BATCH + 1 {
            b_parties.clear();
            for _ in 0..n {
                let mut b = [0u8; 2 * cr_constants::BATCH];
                self.rng.fill_bytes(&mut b);
                let b = unsafe {
                    std::mem::transmute::<[u8; 2 * cr_constants::BATCH], [u16; cr_constants::BATCH]>(
                        b,
                    )
                };
                b_parties.push(b);
            }

            //self.rng.fill_bytes(&mut b1);
            //self.rng.fill_bytes(&mut b2);
            self.rng.fill_bytes(&mut br);
            self.rng.fill_bytes(&mut b_flip);

            // recast to 16 bit
            //let b1 = unsafe {
            //    std::mem::transmute::<[u8; 2 * cr_constants::BATCH], [u16; cr_constants::BATCH]>(b1)
            //};
            //let b2 = unsafe {
            //    std::mem::transmute::<[u8; 2 * cr_constants::BATCH], [u16; cr_constants::BATCH]>(b2)
            //};
            let br = unsafe {
                std::mem::transmute::<[u8; 2 * cr_constants::BATCH], [u16; cr_constants::BATCH]>(br)
            };
            let b_flip = unsafe {
                std::mem::transmute::<[u8; 2 * cr_constants::BATCH], [u16; cr_constants::BATCH]>(
                    b_flip,
                )
            };

            // flip tells us if to flip the result due to qb_err
            let mut flip: [bool; cr_constants::BATCH] = [false; cr_constants::BATCH];
            for (f, v) in flip.iter_mut().zip(b_flip.iter()) {
                *f = *v < threshold;
            }

            let mut output = [0u8; cr_constants::BATCH];

            for i in 0..cr_constants::BATCH {
                //for (e1, e2, r, f, out) in izip!(
                //    b1.iter(),
                //    b2.iter(),
                //    br.iter(),
                //    flip.iter(),
                //    output.iter_mut()
                //) {
                // draw n angles
                let mut a: u32 = 0;
                (0..(n as usize)).for_each(|j| {
                    let index = (b_parties[j][i] % num_angles) as usize;
                    a += angles[index] as u32;
                });
                let a = (a & 127) as u8; // modulo 128
                                         //let a1 = (*e1 % num_angles) as usize;
                                         //let a2 = (*e2 % num_angles) as usize;
                                         // final angle after two parties have applied their modulation
                                         //let a = ((angles[a1] + angles[a2]) as u8) << 1 >> 1;
                                         // result of the measurement
                let mut result = (o[a as usize] < br[i]) as u8;
                // if qber, flip result
                if flip[i] {
                    result ^= 0b1
                };

                let index = (b_parties[k as usize][i] % num_angles) as usize;
                output[i] = angles[index] << 1;
                output[i] |= result;
            }
            v.extend(output.iter());
        }
        //let new_leftover = v.split_off(l);
        if l + self.lfifo_initial as usize > v.len() {
            panic!(
                "in random_correlation: cannot split v of length : {} at new values: l = {} + lfifo_initial = {}",
                v.len(),
                l,
                &self.lfifo_initial
            );
        }
        let new_leftover = v.split_off(l + self.lfifo_initial as usize);
        v.shrink_to_fit();
        Ok((v, new_leftover))
    }
}

// calculate the cosine**2 of all angles
// the state is |psi> = cos(alpha)|0> + sin(alpha)|1>
// where alpha 0..128 corresponds to 0..pi
// note that the angle phi on the Bloch sphere is 2*alpha
fn overlaps() -> [u16; 128] {
    let mut buf = [0u16; 128];
    for (i, elt) in buf.iter_mut().enumerate() {
        *elt = ((i as f32 / 128. * PI).cos().powi(2) * !0u16 as f32) as u16;
    }
    buf
}
