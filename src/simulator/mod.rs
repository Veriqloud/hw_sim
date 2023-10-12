pub mod actor;
pub mod errors;

use errors::Error;
use rand::{distributions::Uniform, Rng};

pub trait Simulator: Send + Sync + Clone + 'static {
    fn generate_raw_keys(self, size: usize, owner: String) -> Result<Keys, Error>;
}

#[derive(Debug)]
pub struct Keys {
    keys: Vec<Vec<u8>>,
    owner: String,
}
pub mod fake {
    use super::*;

    #[derive(Clone)]
    pub struct MockSimu {}

    impl Simulator for MockSimu {
        fn generate_raw_keys(self, size: usize, owner: String) -> Result<Keys, Error> {
            let mut rng = rand::thread_rng();
            let range = Uniform::new(0, 20);
            let key_size = 10;
            let keys = (0..size)
                .map(|_| (0..key_size).map(|_| rng.sample(range)).collect())
                .collect();
            Ok(Keys { keys, owner })
        }
    }
}
