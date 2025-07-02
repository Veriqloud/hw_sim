## Alice (hw + gc + qber)

cargo run --bin hw_sim -- -c ./config/config_hw_sim_alice_local.json

cargo run --bin alice -- --config-path ../config/local_sim_gc_alice.json 100

cargo run --bin alice -- -f ../config/qber_fifo_alice.json -n ../config/qber_net_alice.json 100

## Bob (hw + gc + qber)

cargo run --bin hw_sim -- -c ./config/config_hw_sim_bob_local.json

cargo run --bin bob -- --config-path ../config/local_sim_gc_bob.json

cargo run --bin bob -- -f ../config/local_sim_qber_bob.json



