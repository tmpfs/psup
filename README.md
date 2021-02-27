# psup

A process supervisor that uses JSON-RPC for interprocess communication over Unix domain sockets handled by tokio.

```
cargo build --bin=worker --features=worker && cargo run --features=supervisor
```
