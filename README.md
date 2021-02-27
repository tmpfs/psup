# psup

A non-blocking process supervisor that uses Unix domain sockets for inter-process communication built on [tokio][]. Later support will be added for Windows named pipes.

```
cargo run --example=supervisor --all-features
```

[tokio]: https://docs.rs/tokio/
