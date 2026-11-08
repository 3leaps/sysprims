# sysprims (Python)

Cross-platform process inspection, signal dispatch, session and process-group
containment, timeout execution, and RFC 5545 recurrence (`contract: recurrence/v0`)
primitives backed by the Rust `sysprims` crates via a `cp310-abi3` extension module.

## Capabilities

- **Process inspection & trees**: `proc_get`, `process_list`, `listening_ports`,
  `list_fds`, `descendants`, `kill_descendants`, `guard_step`, `ancestors`, `wait_pid`.
- **Session & self introspection**: `self_pgid`, `self_sid`, `run_setsid`, `run_nohup`.
- **Signals**: `signal_send`, `signal_send_group`, `terminate`, `force_kill`,
  `kill_many`, `terminate_many`, `force_kill_many`.
- **Timeout & containment**: `run_with_timeout`, `terminate_tree`, `spawn_in_group`,
  `spawn_contained`, `ContainedProcess`.
- **Recurrence (`contract: recurrence/v0`)**: `RecurrenceRule`, `RecurrenceRefusal`,
  `RecurrenceQueryError`, `tzdb_version`, `RECURRENCE_CORPUS_DIGEST`, `RECURRENCE_CONTRACT`.

## License

Dual-licensed under MIT OR Apache-2.0.
