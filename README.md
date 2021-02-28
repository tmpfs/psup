# psup

A non-blocking process supervisor that uses Unix domain sockets for inter-process communication built on [tokio][]. Later support will be added for Windows named pipes.

It's purpose is primarily to be used as a libary [psup-impl][] but the [psup][] executable can be used for testing or constrained environments where a lightweight executable could be useful. The statically linked executable is less than 2MB on Linux and could be trimmed down further with a little effort.

```
cargo run --example=supervisor
```

Dual-licensed under MIT and Apacke-2.

[tokio]: https://docs.rs/tokio/
[psup]: https://docs.rs/psup/
[psup-impl]: https://docs.rs/psup-impl/
