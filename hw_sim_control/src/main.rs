use std::{path::PathBuf, process::ExitCode};

use clap::{Args, Parser, Subcommand};
use configs::ipc::{DEFAULT_ALICE_CONTROL_SOCKET_PATH, DEFAULT_BOB_CONTROL_SOCKET_PATH};
use hw_sim_control::send_command_to_pair;
use simulator::runtime_control::CommandRequest;

#[derive(Debug, Parser)]
#[command(about = "Control a local Alice/Bob hardware simulator pair")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
#[command(rename_all = "snake_case")]
enum Command {
    StartAttack {
        #[command(flatten)]
        sockets: SocketOptions,
    },
    StopAttack {
        #[command(flatten)]
        sockets: SocketOptions,
    },
    Recalibrate {
        #[arg(long, help = "Recalibration duration in milliseconds")]
        duration: u64,
        #[command(flatten)]
        sockets: SocketOptions,
    },
}

#[derive(Debug, Args)]
struct SocketOptions {
    #[arg(long, default_value = DEFAULT_ALICE_CONTROL_SOCKET_PATH)]
    alice_socket: PathBuf,
    #[arg(long, default_value = DEFAULT_BOB_CONTROL_SOCKET_PATH)]
    bob_socket: PathBuf,
}

fn main() -> ExitCode {
    let (sockets, request) = match Cli::parse().command {
        Command::StartAttack { sockets } => (sockets, CommandRequest::StartAttack),
        Command::StopAttack { sockets } => (sockets, CommandRequest::StopAttack),
        Command::Recalibrate { duration, sockets } => (
            sockets,
            CommandRequest::Pause {
                duration_ms: duration,
            },
        ),
    };

    match send_command_to_pair(&sockets.alice_socket, &sockets.bob_socket, &request) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command};
    use clap::Parser;
    use configs::ipc::{DEFAULT_ALICE_CONTROL_SOCKET_PATH, DEFAULT_BOB_CONTROL_SOCKET_PATH};
    use std::path::Path;

    #[test]
    fn parses_recalibration_with_default_sockets() {
        let cli =
            Cli::try_parse_from(["hw_sim_control", "recalibrate", "--duration", "200"]).unwrap();

        let Command::Recalibrate { duration, sockets } = cli.command else {
            panic!("expected recalibrate command");
        };
        assert_eq!(duration, 200);
        assert_eq!(
            sockets.alice_socket,
            Path::new(DEFAULT_ALICE_CONTROL_SOCKET_PATH)
        );
        assert_eq!(
            sockets.bob_socket,
            Path::new(DEFAULT_BOB_CONTROL_SOCKET_PATH)
        );
    }

    #[test]
    fn command_names_use_underscores() {
        assert!(matches!(
            Cli::try_parse_from(["hw_sim_control", "start_attack"])
                .unwrap()
                .command,
            Command::StartAttack { .. }
        ));
        assert!(Cli::try_parse_from(["hw_sim_control", "start-attack"]).is_err());
    }
}
