use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct Configuration {
    pub command_file_path: String,      // Should be /dev/cmd
    pub angle_file_path: String,        // Should be /dev/c2h_angles
    pub click_result_file_path: String, // Should be /dev/c2h_click_results
    pub gc_file_path: String,           // Should be /dev/h2c_gc
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            command_file_path: String::from_str("/dev/cmd").unwrap(),
            angle_file_path: String::from_str("/dev/c2h_angles").unwrap(),
            click_result_file_path: String::from_str("/dev/c2h_click_results").unwrap(),
            gc_file_path: String::from_str("/dev/h2c_gc").unwrap(),
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
                command_file_path: "/dev/cmd_test".to_owned(),
                angle_file_path: "/dev/c2h_angles_test".to_owned(),
                click_result_file_path: "/dev/c2h_click_results_test".to_owned(),
                gc_file_path: "/dev/h2c_gc_test".to_owned()
            },
            config_input
        );
    }
}
