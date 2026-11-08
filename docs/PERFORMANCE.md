# Performance notes

The benchmark crate is intentionally small. It measures the same transactional copy path used by
the core instead of a separate synthetic implementation.

Run a local copy sample with:

```sh
cargo run --release -p searvorn-bench -- 64
```

The argument is the fixture size in MiB. The output currently reports bytes copied, elapsed time,
throughput, and whether the destination commit was atomic and directory-synced.

For comparisons, keep these variables with the result:

- commit SHA and Rust toolchain;
- device and Android/kernel version;
- storage source and destination;
- fixture size;
- power/thermal state;
- copy chunk size.

Hosted CI only runs a 1 MiB smoke test. It must not enforce throughput thresholds because shared
runners are too noisy for meaningful regression gates.

Device benchmarks will eventually cover at least 10 MiB, 100 MiB, 500 MiB, and 1 GiB inputs.
Performance changes should be judged together with peak memory and CPU time; raw throughput alone
is not enough.
