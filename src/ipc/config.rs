use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct Configuration {
    pub unix_socket_path: String,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            unix_socket_path: String::from_str("./Node2HW.sock").unwrap(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn valid_config() {
        let config_input_string =
            std::fs::read_to_string("src/ipc/test_data/valid_config.json").unwrap();

        let config_input: crate::ipc::config::Configuration =
            serde_json::from_str(&config_input_string).unwrap();

        assert_eq!(
            crate::ipc::config::Configuration {
                unix_socket_path: "path as str".to_owned()
            },
            config_input
        );
    }
}
