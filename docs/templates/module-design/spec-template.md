---
title: "{{MODULE}} Module Spec"
module: "{{MODULE}}"
version: "0.1"
status: "Draft"
last_updated: "{{DATE}}"
adr_refs: []
---

<!-- TEMPLATE: Replace all {{PLACEHOLDER}} values and remove these comments -->

# {{MODULE}} Module Spec

## 1) Overview

**Purpose:** <!-- Brief description of what this module does -->

**In scope (v0.1.0):**
- <!-- Feature 1 -->
- <!-- Feature 2 -->

**Out of scope (v0.1.0):**
- <!-- Deferred feature 1 -->

**Supported platforms:**
- Linux (x64, musl + glibc)
- macOS (x64, arm64)
- Windows (x64)

**Known limitations:**
- <!-- Platform-specific limitation, if any -->

## 2) Normative References

### Primary Specifications

<!-- List authoritative specifications this module implements -->

| Source | URL | Notes |
|--------|-----|-------|
| <!-- e.g., POSIX ps --> | <!-- URL --> | <!-- What we use from it --> |

### Platform Implementation References

| Platform | API | Reference |
|----------|-----|-----------|
| Linux | <!-- e.g., /proc --> | <!-- man page or doc URL --> |
| macOS | <!-- e.g., libproc --> | <!-- doc URL --> |
| Windows | <!-- e.g., Toolhelp32 --> | <!-- MSDN URL --> |

## 3) Literal Interface Reference

<!-- If implementing a standard interface, document it here -->

### Synopsis

```
<!-- Command synopsis or function signature from spec -->
```

### Behavior Notes

<!-- Key behavioral details from the specification -->

## 4) sysprims Required Interface (Rust)

### 4.1 Types

```rust
// <!-- Core types with actual signatures -->
```

### 4.2 Functions

```rust
// <!-- Public functions with actual signatures and doc comments -->
```

### 4.3 Error Handling

Per ADR-0008, this module returns these errors:

| Error | Condition |
|-------|-----------|
| `InvalidArgument` | <!-- When returned --> |
| `NotFound` | <!-- When returned --> |
| `PermissionDenied` | <!-- When returned --> |
| `NotSupported` | <!-- When returned --> |

### 4.4 Platform Behavior

| Feature | Linux | macOS | Windows |
|---------|-------|-------|---------|
| <!-- Feature --> | <!-- Behavior --> | <!-- Behavior --> | <!-- Behavior --> |

## 5) CLI Contract

### Synopsis

```
sysprims {{SUBCOMMAND}} [OPTIONS] [ARGS...]
```

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `--json` | Output as JSON with schema_id | false |
| <!-- Other options --> | <!-- Description --> | <!-- Default --> |

### Exit Codes

| Condition | Exit Code |
|-----------|-----------|
| Success | 0 |
| <!-- Other conditions --> | <!-- Code --> |

### Output Formats

**Human readable (default):**
```
<!-- Example output -->
```

**JSON (`--json`):**
```json
{
  "schema_id": "https://schemas.3leaps.dev/sysprims/{{module}}/v1.0.0/{{type}}.schema.json",
  // ...
}
```

## 6) FFI Contract

### C ABI Functions

```c
// <!-- FFI function declarations -->
SysprimsErrorCode sysprims_{{module}}_{{function}}(...);
```

### Memory Ownership

- All returned strings must be freed with `sysprims_free_string()`
- <!-- Other ownership rules -->

### Thread Safety

- <!-- Thread safety guarantees -->

## 7) Traceability Matrix

| Requirement | Reference | API | CLI | Tests | Evidence |
|-------------|-----------|-----|-----|-------|----------|
| <!-- Req --> | <!-- Spec section --> | `function()` | `sysprims cmd` | `test_*` | CI link |

---

*Spec version: 0.1*
*Last updated: {{DATE}}*
