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

- **Rust Toolchain**: Ensure you have Rust and Cargo installed. You can get them from rustup.rs.
- **Git**: Required for cloning the repositories.
- **Build Tools**: A C compiler and build tools are needed (`build-essential` on Debian/Ubuntu).

## 2. Installation

First, clone the necessary repositories and build the binaries for each component.

Clone and build hw_sim :
```
git clone --branch master git@github.com:Veriqloud/hw_sim.git
cd hw_sim && cargo build --release && cd ..
```

Clone and build gc and qber :
```
git clone --branch master git@github.com:Veriqloud/kiwi_hw_control.git
cd kiwi_hw_control/gc && cargo build --release
cd ../qber && cargo build --release && cd ..
```

## 3. Configuration

### For Alice
Here is an example of hw_sim configuration file for Alice :
```
{
  "backend_config": {
    "angles": [
      0,
      32,
      64,
      96
    ],
    "seed": 42,
    "eta": 0.0,
    "qberr": 0.05,
    "pulse_distance": 1e-8
  },
  "ipc_config": {
    "command_path": "/tmp/fpga_alice",
    "angle_file_path": "/tmp/gc_alice_angle.fifo",
    "gc_read_file_path": "/tmp/gc_alice_gc.fifo",
    "hw_params_file_path": "/tmp/hw_params_alice.txt"
  },
  "log_level": "Info"
}
```

Here is an example of gc configuration file for Alice :
```
{
  "player": {
    "Alice": {
      "fifo": {
        "command_socket_path": "/tmp/gc_alice_command.socket",
        "gc_file_path": "/tmp/gc_alice_gc.fifo"
      },
      "network": {
        "ip_gc": "127.0.0.1:53948"
      }
    }
  },
  "current_hw_parameters_file_path": "/tmp/hw_params_alice.txt",
  "fpga_start_socket_path": "/tmp/fpga_alice",
  "log_level": "Info",
  "ignore_gcr_timeout": true
}
```

Here is an example of qber configuration file for Alice :
```
{
  "ip_bob": "127.0.0.1:58000",
  "angle_file_path": "/tmp/gc_alice_angle.fifo",
  "command_socket_path": "/tmp/gc_alice_command.socket"
}
```


### For Bob
Here is an example of hw_sim configuration file for Bob :
```
{
  "backend_config": {
    "angles": [
      0,
      32,
      64,
      96
    ],
    "seed": 42,
    "eta": 0.0,
    "qberr": 0.05,
    "pulse_distance": 1e-8
  },
  "ipc_config": {
    "command_path": "/tmp/fpga_bob",
    "angle_file_path": "/tmp/gc_bob_angle.fifo",
    "gcr_file_path": "/tmp/gc_bob_gcr.fifo",
    "gc_read_file_path": "/tmp/gc_bob_gc.fifo",
    "hw_params_file_path": "/tmp/hw_params_bob.txt"
  },
  "log_level": "Info"
}
```

Here is an example of gc configuration file for Bob:
```
{
  "player": {
    "Bob": {
      "fifo": {
        "gcr_file_path": "/tmp/gc_bob_gcr.fifo",
        "gc_file_path": "/tmp/gc_bob_gc.fifo",
        "click_result_file_path": "/tmp/gc_bob_click_result.fifo",
        "gcuser_file_path": ""
      },
      "network": {
        "ip_gc": "127.0.0.1:53948"
      }
    }
  },
  "current_hw_parameters_file_path": "/tmp/hw_params_bob.txt",
  "fpga_start_socket_path": "/tmp/fpga_bob",
  "log_level": "Info",
  "ignore_gcr_timeout": true
}
```

Here is an example of qber configuration file for Bob :
```
{
  "ip_listen": "127.0.0.1:58000",
  "angle_file_path": "/tmp/gc_bob_angle.fifo",
  "click_result_file_path": "/tmp/gc_bob_click_result.fifo"
}
```

## Run the stack locally

First, we run both simulators for Alice and Bob.
```
cd ../hw_sim && cargo run --bin simulator -- --config-path PATH_TO_HW_ALICE_CONFIG
cargo run --bin simulator -- --config-path PATH_TO_HW_BOB_CONFIG
```

Then, we run both gc for Alice and Bob.
```
cd ../kiwi_hw_control/gc && cargo run --bin alice -- -c PATH_TO_GC_ALICE_CONFIG
cargo run --bin bob -- -c PATH_TO_GC_BOB_CONFIG
```


Finally, we run both qber for Alice and Bob.
```
cd ../qber && cargo run --bin bob -- -c PATH_TO_QBER_BOB_CONFIG
cargo run --bin alice -- -c PATH_TO_QBER_ALICE_CONFIG 6400
```
