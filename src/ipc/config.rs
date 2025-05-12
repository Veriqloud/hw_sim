use std::{path::Path, str::FromStr};

use serde::{Deserialize, Serialize};
use snafu::ResultExt;

use crate::config::errors::{Error, PathNotExistSnafu};

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct Configuration {
    pub command_socket_path: String,
    pub angle_socket_path: String,
    pub click_result_socket_path: String,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            command_socket_path: String::from_str("./xdma0_user").unwrap(),
            angle_socket_path: String::from_str("./xdma0_c2h3_3").unwrap(),
            click_result_socket_path: String::from_str("./click_result").unwrap(),
        }
    }
}

impl Configuration {
    pub fn check_all_fields_exist(&self) -> Result<(), Error> {
        let fields = [
            &self.command_socket_path,
            &self.angle_socket_path,
            &self.click_result_socket_path,
        ];

        for field in fields.iter() {
            let path = Path::new(field);

            if path.exists() {
                std::fs::remove_file(path).context(PathNotExistSnafu {
                    path: field.to_string(),
                })?;
            }
        }
        Ok(())
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
                command_socket_path: "path as str".to_owned(),
                angle_socket_path: "another path as str".to_owned(),
                click_result_socket_path: "yet another path as str".to_owned()
            },
            config_input
        );
    }
}
