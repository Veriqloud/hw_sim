use std::{fs, ops::RangeInclusive, path::Path};

use clap::Args;

/// Script to generate KMS bundles for a given network.  
#[derive(clap::Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct CliArgs {
    #[command(flatten)]
    pub conf: ConfigArgs,

    /// Set a custom logs location. Expects a directory.
    #[arg(
        short='l',
        long="logs-location", 
        value_parser = path_is_dir_exists_and_writable,
        default_value_t = String::from("/tmp")
    )]
    pub logs_location: String,
}

#[derive(Args, Clone, Debug)]
#[group(required = true, multiple = false)]
pub struct ConfigArgs {
    /// Specify the IPC socket path, without the need for a config file.
    #[arg(short='i', long="ipc_socket", value_parser = path_exists)]
    pub ipc_socket: Option<String>,

    /// Provide a config file.
    #[arg(short='c', long="config-path", value_parser = path_exists)]
    pub config_path: Option<String>,
}

// Needs to return a String because it can be used on its own to validate a path.
fn path_exists(path: &str) -> Result<String, String> {
    if let Ok(exists) = Path::new(path).try_exists() {
        if exists {
            return Ok(String::from(path));
        }
    }

    Err("path provided as argument is invalid or doesn't exist".to_owned())
}

// Doesn't need to return a String because it is not used on its own to validate a path.
fn path_is_writable(path: &str) -> Result<(), String> {
    let md = match fs::metadata(path) {
        Ok(md) => md,
        Err(e) => {
            return Err(e.to_string());
        }
    };

    let permissions = md.permissions();

    if permissions.readonly() {
        return Err("path is not writable !\nPath:{path}".to_string());
    }

    Ok(())
}

fn path_is_directory(path: &str) -> Result<(), String> {
    if !Path::new(path).is_dir() {
        return Err("path is not a directory !".to_string());
    }

    Ok(())
}
fn path_is_dir_exists_and_writable(path: &str) -> Result<String, String> {
    let path_string = path_exists(path)?;
    path_is_directory(path)?;
    path_is_writable(path)?;
    Ok(path_string)
}
