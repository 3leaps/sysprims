# Module Design Documentation

This directory contains design specifications for sysprims modules per the [Module Design SOP](../standards/module-design-sop.md).

## Purpose

These documents serve to:

1. **Show our work** — Document how APIs were designed from specifications
2. **Enable traceability** — Map requirements to implementation to tests
3. **Support review** — Give reviewers clear acceptance criteria
4. **Maintain provenance** — Record sources consulted for each module

## Module Index

| Module                                  | Purpose                                         | Status          |
| --------------------------------------- | ----------------------------------------------- | --------------- |
| [sysprims-proc](./sysprims-proc/)       | Process enumeration, wait_pid                   | Complete (v1.1) |
| [sysprims-signal](./sysprims-signal/)   | Signal dispatch                                 | Complete        |
| [sysprims-timeout](./sysprims-timeout/) | Process timeout, spawn_in_group, terminate_tree | Complete (v1.1) |
| [sysprims-session](./sysprims-session/) | Session management                              | Complete        |
| [sysprims-cli](./sysprims-cli/)         | CLI wrapper                                     | Complete        |

## v0.1.6 Changes

The following specs were updated for v0.1.6:

- **sysprims-proc v1.1**: Added `start_time_unix_ms`, `exe_path` fields and `wait_pid()` function
- **sysprims-timeout v1.1**: Added `spawn_in_group()` and `terminate_tree()` functions

## Document Structure

Each module directory contains:

| Document                 | Purpose                               |
| ------------------------ | ------------------------------------- |
| `*-spec.md`              | API contract and design rationale     |
| `*-equivalence-tests.md` | Test protocol and acceptance criteria |
| `*-compliance.md`        | Evidence that requirements are met    |
| `*-provenance.md`        | Sources consulted and avoided         |

## For Reviewers

### Quick Start

1. Read the SOP: [`../standards/module-design-sop.md`](../standards/module-design-sop.md)
2. For each module, review in order: spec → equivalence tests → compliance → provenance

### Key Questions

| Question                       | Where to Look                         |
| ------------------------------ | ------------------------------------- |
| What does this module do?      | `*-spec.md` §1 (Overview)             |
| What standards does it follow? | `*-spec.md` §2 (Normative References) |
| What's the API contract?       | `*-spec.md` §4 (Rust Interface)       |
| How is correctness verified?   | `*-equivalence-tests.md`              |
| What sources were used?        | `*-provenance.md`                     |

## Templates

Templates for creating new module documentation are in [`../templates/module-design/`](../templates/module-design/).

---

_Last updated: 2026-01-25_
