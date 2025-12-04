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
