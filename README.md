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
{"command":"synchronize","batches_to_discard":4}
{"command":"resume"}
```

The three recalibration messages form one coordinated exchange. Use `hw_sim_control recalibrate`
instead of sending them manually. `pause` is handled at batch boundaries; it does not interrupt an
in-progress FIFO read/write.

Each command receives a newline-delimited JSON response:

```json
{"status":"ok"}
{"status":"ok","progress":{"event_count":102400,"batch_pulse_count":10240}}
{"status":"error","message":"pause already pending or running"}
```

### Local control client

The `hw_sim_control` binary sends a command to both members of a local Alice/Bob pair:

```bash
cargo run -p hw_sim_control -- start_attack
cargo run -p hw_sim_control -- stop_attack
cargo run -p hw_sim_control -- recalibrate --duration 5000
```

It uses the default sockets listed above. Custom local paths can be selected with
`--alice-socket PATH` and `--bob-socket PATH`. Commands are sent to both sockets
concurrently, and the process exits with an error if either simulator rejects the command or
cannot be reached.

The client only supports local Unix sockets. It does not expose TCP, IP address, or port
options.

At startup, the simulator removes its stale idle acknowledgement, starts the filesystem watcher, and creates the configured hardware-ready file.

The recalibration command pauses both simulators, compares their generation progress, and makes the
lagging simulator generate and discard the missing batches. Once both seeded random streams are
synchronized, the simulators wait through the watcher for the idle acknowledgement created by `gc`,
sleep for the requested duration, reset the IPC FIFOs, and recreate their hardware-ready files. A
stale idle acknowledgement is removed before starting, but the new one is left for `gc` to remove
when the node resumes.

The handshake paths must match between each `hw_sim` and `gc` pair:

| `hw_sim` IPC configuration | `gc` configuration |
| --- | --- |
| `qkd_ready_path` | `ready_flag_path` |
| `node_idle_path` | `node_idle_flag_path` |

When Alice and Bob share the same filesystem, use player-specific paths, for example:

| Player | Hardware ready | Node idle |
| --- | --- | --- |
| Alice | `/tmp/qkd_ready_alice` | `/tmp/node_idle_alice` |
| Bob | `/tmp/qkd_ready_bob` | `/tmp/node_idle_bob` |

Do not share one hardware-ready file between two independent simulator processes: the first simulator to finish could advertise readiness while the other is still recalibrating.

To recalibrate a local Alice/Bob pair, run `hw_sim_control recalibrate`. Each consuming node must
also enable `support_recalibration` so it polls `gc`, closes its FIFO readers while the hardware is
unavailable, and reopens the recreated FIFOs afterwards.

This filesystem handshake belongs to the standalone `simulator` runtime. A binary embedding `sim_lib` directly gets batch generation, session lifecycle, and attack controls, but must provide its own recalibration orchestration if it needs to simulate runtime pauses.
