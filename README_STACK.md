## Alice (hw + gc + qber)

cargo run --bin simulator -- -c ./config/config_hw_sim_alice_local.json

cargo run --bin alice -- --config-path ../config/local_sim_gc_alice.json 100

cargo run --bin alice -- -f ../config/qber_fifo_alice.json -n ../config/qber_net_alice.json 100

## Bob (hw + gc + qber)

cargo run --bin simulator -- -c ./config/config_hw_sim_bob_local.json

cargo run --bin bob -- --config-path ../config/local_sim_gc_bob.json

cargo run --bin bob -- -f ../config/local_sim_qber_bob.json


# Control hw sim output
You can edit the config, and setup these values: 
- Pulse distance: the time between two photons emissions
- eta: channel efficiency. 1 means every photon is transmitted

The way you use this is by choosing a number of "events per second", an event being a photon generation, so a single potential bit for the key (potential because of all the post processing; photon angle + basis).
That number is: eta/pulse_distance.
So for example, if you take eta = 0.1 and pulse_distance = 10e-5, then you get: 10e-1/10e-5 = 10e4 events per second, that will result in 5kb/s produced by the sim (because all bytes contain 2 events for the angles).


