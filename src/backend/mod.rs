pub mod actor;
pub mod errors;
pub mod protocols;
pub mod role;
pub mod simulation;

use libhardware::Backend;
use rand::{distributions::Uniform, Rng};

pub trait BytesGenerator: Backend + Send + Sync + Clone + 'static {}

pub mod fake {
    use super::*;

    #[derive(Clone)]
    pub struct MockSimu {}

    impl Backend for MockSimu {
        fn get_gcsafe(&mut self) -> u64 {
            todo!()
        }

        fn get_global_counter(&mut self) -> Option<u64> {
            todo!()
        }

        fn read_angles(&mut self) -> Result<Vec<u8>, libhardware::HardwareError> {
            let mut rng = rand::thread_rng();
            let range = Uniform::new(0, 20);
            let key_size = 10;
            let data = (0..key_size).map(|_| rng.sample(range)).collect();
            Ok(data)
        }

        fn set_modulator_state(
            &mut self,
            _modulator_state: libhardware::ModulatorState,
            _at_global_counter: u64,
        ) -> Result<u32, libhardware::HardwareError> {
            todo!()
        }
    }
    impl BytesGenerator for MockSimu {}
}
