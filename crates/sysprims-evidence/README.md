# sysprims-evidence

Portable, `#![forbid(unsafe_code)]` Rust implementation of the
`identity-evidence/v0` wire admission, report envelope validation,
`local-process-session:v0` contract-fit checks, and
`native-process-observation:v0` incarnation/context comparison, live freshness
admission, context preimage derivations, and fail-closed session projection.

This crate performs no OS process syscalls and links no platform collector
dependencies; live native collectors and private continuity handles live in
`sysprims-proc::observation`.

## License

MIT OR Apache-2.0
