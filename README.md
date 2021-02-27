# psup

A non-blocking process supervisor that uses Unix domain sockets for inter-process communication built on [tokio][]. Later support will be added for Windows named pipes.

```
cargo build --bin=worker --features=worker && cargo run --features=supervisor
```

[tokio]: https://docs.rs/tokio/
