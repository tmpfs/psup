# psup

A non-blocking process supervisor that uses Unix domain sockets for inter-process communication built on [tokio][]. Later support will be added for Windows named pipes.

It's purpose is primarily to be used as a libary [psup-impl][] but the [psup][] executable can be used for testing or constrained environments where a lightweight executable could be useful. The statically linked release executable is ~2MB with symbols stripped on Linux and could be trimmed down further with a little effort.

Example communicating using JSON RPC:

```
cargo run --example=supervisor
```

An example that uses the supervisor control channel to explicitly shutdown a daemon worker process:

```
cargo run --example=shutdown
```

To test the daemon and respawn logic:

```
cargo run -- sample.toml
kill <PID>
```

Build a release:

```
cargo build --release && strip target/release/psup
```

Dual-licensed under MIT and Apacke-2.

[tokio]: https://docs.rs/tokio/
[psup]: https://docs.rs/psup/
[psup-impl]: https://docs.rs/psup-impl/
