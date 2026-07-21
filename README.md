# Hardware Simulator

This project contains a simulator of the VeriQloud QKD hardware, along with a controller to manage it for test purposes.
The simulator mimics a quantum key distribution protocol between Alice (the source) and Bob (the detector). In this simulator, no communication is used to have the expected correlation but instead a preshared seed. This simulator is not intended to be used in any production environment, and only used for test and debug purposes.


## Building the Binaries

The project is built using Rust and Cargo. The binaries can be built for release with the following command:

```bash
cargo build --release
```


### Configuration

The simulator expects a configuration file, likely in JSON format. The configuration file specifies various parameters for the simulation. While the exact structure is defined in the `configs` crate, it includes:

*   Paths to input and output files
*   Hardware parameters (qber, pulse_rate, etc.)
*   Logging configuration

The configuration files can be generated using the [QLine Auto Setup Tool](https://github.com/Veriqloud/qline_backend/tree/master/auto_setup).

This tool simplifies the setup and deployment of Alice (source) and Bob (detector) nodes by automatically generating all necessary configuration files and run scripts.


### Example

To run the simulator for Alice :
```bash
cargo run --bin simulator -- --config-path ~/.config/qline/alice/hw_sim_config.json
```

The `simu_controller` is a command-line tool provided to test the `simulator`. It acts as a client, sending commands and data to the simulator to verify its functionality and performance. 

To run the simu_controller for Alice :
```bash
cargo run --bin simu_controller -- --config-path ~/.config/qline/alice/hw_sim_config.json alice 1000
```

## Simulating QKD Attacks

The simulator includes a built-in feature to simulate an attack on the quantum channel. When active, this mode forces the Quantum Bit Error Rate (QBER) to **50%**, rendering the key exchange insecure.

This feature is controlled through the simulator runtime control socket.

## Runtime Control Socket

When the simulator starts, it opens a Unix socket for newline-delimited JSON commands:

* Alice default: `/tmp/hw_sim_alice_control.socket`
* Bob default: `/tmp/hw_sim_bob_control.socket`

The path can be overridden with `control_socket_path` in `ipc_config`.

Supported commands:

```json
{"command":"start_attack"}
{"command":"stop_attack"}
{"command":"pause","duration_ms":5000}
```

`pause` is handled at batch boundaries; it does not interrupt an in-progress FIFO read/write.

Each command receives a newline-delimited JSON response:

```json
{"status":"ok"}
{"status":"error","message":"pause already pending or running"}
```

At startup, the simulator removes its stale idle acknowledgement, starts the filesystem watcher, and creates `/tmp/qkd_ready`.

The `pause` command starts a simulated recalibration. The simulator removes the shared `/tmp/qkd_ready` flag, waits through the watcher for the player-specific idle acknowledgement created by `gc` (`/tmp/node_idle_alice` or `/tmp/node_idle_bob`), sleeps for the requested duration, resets the IPC FIFOs, and recreates `/tmp/qkd_ready`. It removes a stale idle acknowledgement before starting, but leaves the new one for `gc` to remove when the node resumes. These paths can be overridden with `qkd_ready_path` and `node_idle_path` in `ipc_config`.
