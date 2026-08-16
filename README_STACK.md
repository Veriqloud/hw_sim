# Running the VeriQloud QKD Simulator Stack

This guide describes the steps to build and run the VeriQloud QKD (Quantum Key Distribution) simulator stack. This stack allows you to simulate a full QKD experiment and provides a platform for developing and testing your own QKD applications.

The full stack is composed of three main programs, which can run on the same machine or across different machines:

- VQ hardware simulator made of two projects [hw_sim](https://github.com/Veriqloud/hw_sim) and [gc](https://github.com/Veriqloud/kiwi_hw_control/tree/master/gc)
  combined together to emulate the real quantum hardware and its control software.
  - `hw_sim`: Emulates the low-level quantum hardware for both Alice and Bob.
  - `gc` (Global Counter): for the real hardware, this program helps with the synchronization of players' events. We keep it in our software stack to be able to replace easily the real hardware with its emulation.
- An application that consumes the data from the simulator. In this guide, we use
  qber as an example application to estimate the Qubit Error Rate (QBER).

The goal of this guide is to show you how to run the simulator (`hw_sim` + `gc`) with the `qber` application. Once you are familiar with this setup, you can replace `qber` with your own application to interact with the QKD hardware simulator.

## 1. Prerequisites

For running with Docker (recommended):
- **Docker**: Ensure you have a recent version of Docker installed.
- **Docker Compose**: The `compose` plugin for Docker is required.

For running manually (for development):
- **Rust Toolchain**: Ensure you have Rust and Cargo installed (`rustup.rs`).
- **Git**: Required for cloning the repositories.
- **Build Tools**: A C compiler and build tools (`build-essential` on Debian/Ubuntu).

## 2. Configuration

The stack requires several configuration files for each component (hw_sim, gc, qber) and for each player (Alice, Bob).

Example configuration files are provided in the `config_files/` directory, organized by player:

```
config_files/
├── alice/
│   ├── gc_config.json
│   ├── hw_sim_config.json
│   └── qber_config.json
└── bob/
    ├── gc_config.json
    ├── hw_sim_config.json
    └── qber_config.json
```

These files are set up for baseline local communication using FIFOs and sockets in the `/tmp` directory and are ready to be used with the provided `docker-compose.yml`. They do not currently declare the recalibration handshake paths described below.

### Runtime recalibration paths

Recalibration uses two local files for each player. Their paths must agree between that player's `hw_sim` and `gc` configurations:

| Purpose | `hw_sim` IPC configuration | `gc` configuration |
| --- | --- | --- |
| Hardware availability | `qkd_ready_path` | `ready_flag_path` |
| Node idle acknowledgement | `node_idle_path` | `node_idle_flag_path` |

Alice and Bob must use different paths when they share one filesystem. Recommended local paths are `/tmp/qkd_ready_alice` and `/tmp/node_idle_alice` for Alice, and `/tmp/qkd_ready_bob` and `/tmp/node_idle_bob` for Bob. Reusing `/tmp/qkd_ready` for both independent simulator processes is unsafe because either process could recreate it before the other has completed its recalibration.

When the players run on separate machines or in containers with separate `/tmp` filesystems, they may use identical local path names. Each consuming node must set `support_recalibration` to `true`, and a simulated pair must receive the `pause` command on both `hw_sim` control sockets.

## 3. Running the Stack

### Using Docker Compose (Recommended)

The easiest way to run the full simulation stack is with Docker Compose. This will build the necessary container images and run all services with the correct configuration.

To start the stack, run:
```bash
docker compose up
```

You will see logs from all the services (`hw_sim`, `gc`, and `qber` for both Alice and Bob).

To stop the services and remove the containers, press `Ctrl+C` and then run:
```bash
docker compose down
```

### Running Manually (for Development)

If you prefer to run the components manually for development or debugging, you can build and run them directly using Cargo.

**1. Build the Binaries**
```bash
# Clone and build hw_sim
git clone --branch master https://github.com/Veriqloud/hw_sim.git
cd hw_sim && cargo build --release && cd ..

# Clone and build gc and qber
git clone --branch master https://github.com/Veriqloud/kiwi_hw_control.git
cd kiwi_hw_control/gc && cargo build --release && cd ..
cd qber && cargo build --release && cd ../..
```

**2. Run the Services**

You will need to open multiple terminal windows to run each component. Make sure to replace the placeholder paths with the actual paths to the configuration files in the `config_files` directory.

*In terminal 1 (hw_sim Alice):*
```bash
cd hw_sim && cargo run --bin simulator -- --config-path ../config_files/alice/hw_sim_config.json
```

*In terminal 2 (hw_sim Bob):*
```bash
cd hw_sim && cargo run --bin simulator -- --config-path ../config_files/bob/hw_sim_config.json
```

*In terminal 3 (gc Alice):*
```bash
cd kiwi_hw_control/gc && cargo run --bin alice -- -c ../../config_files/alice/gc_config.json
```

*In terminal 4 (gc Bob):*
```bash
cd kiwi_hw_control/gc && cargo run --bin bob -- -c ../../config_files/bob/gc_config.json
```

*In terminal 5 (qber Bob):*
```bash
cd kiwi_hw_control/qber && cargo run --bin bob -- -c ../../config_files/bob/qber_config.json
```

*In terminal 6 (qber Alice):*
```bash
cd kiwi_hw_control/qber && cargo run --bin alice -- -c ../../config_files/alice/qber_config.json 6400
```

## 4. Using Your Own Application

The stack is designed to be modular, allowing you to replace the example `qber` application with your own custom application to process data from the simulated hardware.

Your application will typically interact with the `gc` (Global Counter) through the Start/Stop command. The generated bytes will be available at the "angle_file_path" specified
at the `hw_sim_config.json`.

Here’s how to integrate your own application, which we'll call `my_app`.

### 1. Create a Dockerfile for Your Application

The simplest approach is to copy `Dockerfile.qber` and modify it to build your application's binaries.


### 2. Update Docker Compose

Modify `docker-compose.yml` to build and run your application instead of `qber`. You'll need to update the `qber_alice` and `qber_bob` services.

1.  Change the `build` context to use your new `Dockerfile.my_app`.
2.  Update the `command` to execute your application's binaries.
3.  If your application uses different configuration file names, update the `volumes` to mount the correct files.


### 3. Add Configuration Files

Place the configuration files for your application (e.g., `my_app_config.json`) in the `config_files/alice` and `config_files/bob` directories.

### 4. Run the New Stack

With the changes in place, you can run the entire stack, now with your custom application, using the same Docker Compose command:

```bash
docker compose up --build
```
