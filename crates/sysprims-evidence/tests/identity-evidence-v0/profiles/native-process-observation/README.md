# Native process observation — provider profile

**Status:** proposed draft provider profile, not a qualified OS adapter or an
authorization credential. `profile.json` pins this specification and
`evidence.schema.json`; evidence descriptors and payloads pin `profile.json` by
exact SHA-256 revision.

## Purpose and authority boundary

Record a read-only OS observation of one process incarnation and its selected
execution-context facts so local or downstream verifiers can distinguish live
continuity from process exit (`not-present`), zombie retention (`zombie`), PID
reuse, namespace mismatch, context drift, or lost/indeterminate visibility
(`inaccessible`, `error`, `unknown`).

Observation-only reports using this profile omit `evaluator`, keep `assessments`
empty, and cite the canonical payload digest from an `os-observation` evidence
descriptor whose `collected_at` equals `payload.observed_at` and whose
`format_profile` equals `payload.profile` and the accepted `profile.json`
reference. An observation payload does not authenticate the caller, prove
physical-machine uniqueness, exclude VM or live-snapshot cloning unless
established by host-scope policy, separate processes sharing an account, or
grant signal or launch authority.

Live OS continuity handles (`pidfd` on Linux, `kqueue` `EVFILT_PROC` `NOTE_EXIT`
on macOS, or equivalent non-destructive OS synchronization handles) are private
in-memory adapter resources. They require no signal permission for acquisition,
expose no signal API under this profile, and are **never serialized** into
portable JSON payloads or treated as bearer proof across process boundaries.
Adapter probe outcomes for live handle mechanisms may be reported in the report
`capabilities` array.

## Fact status vocabulary and report capability mapping

Payload fact wrappers use a seven-state `status` vocabulary with a strict 1:1
bijection to `report.schema.json` `capabilities[].status`:

| Payload fact `status` | Report `capabilities[].status` | Value / digest / derivation fields | `reason` field | Required meaning                                                                                                        |
| --------------------- | ------------------------------ | ---------------------------------- | -------------- | ----------------------------------------------------------------------------------------------------------------------- |
| `observed`            | `available`                    | Required                           | Optional       | Probe succeeded and passed pre/post incarnation bracketing                                                              |
| `not-present`         | `not-present`                  | Forbidden                          | Required       | Verified observer view establishes that the target PID or optional kernel facility does not exist (`ENOENT` / `ESRCH`)  |
| `inaccessible`        | `inaccessible`                 | Forbidden                          | Required       | Kernel or OS access control denied visibility (`EPERM`, `EACCES`, `ptrace_may_access`, `hidepid=invisible`)             |
| `unsupported`         | `unsupported`                  | Forbidden                          | Required       | Platform family or adapter cell does not implement the concept (for example PID/time namespaces on macOS)               |
| `not-probed`          | `not-probed`                   | Forbidden                          | Required       | Fact omitted by default opt-in disclosure policy or explicit caller probe budget                                        |
| `error`               | `error`                        | Forbidden                          | Required       | Unexpected OS error, mount/thread violation, malformed kernel surface, or bracketing race prevented a trustworthy read  |
| `unknown`             | `unknown`                      | Forbidden                          | Required       | Visibility is indeterminate under the current mount, `hidepid` policy, container or observer scope                      |

Adapters must preserve `not-present`, `inaccessible`, `error` and `unknown`
distinctly and never collapse `error` or `unknown` into `inaccessible`,
`unsupported`, `not-present`, `zombie` or process exit:

- **Target absent / exited (`not-present`):** When a verified collector view
  establishes that no process exists at `pid` (`sysctl("kern.proc.pid", pid)`
  returns `ESRCH` or 0 bytes on macOS; `openat2(proc_fd, "<pid>", ...)` returns
  `ENOENT` on a verified Linux `procfs` without `hidepid` invisibility), the
  collector records `process_state.status = "not-present"` and
  `start_identity.status = "not-present"` (with `reason: "target-pid-absent"`).
  If `hidepid` may conceal live processes as `ENOENT`, the collector must record
  `unknown` (or `inaccessible`) rather than `not-present`.
- **Zombie state (`zombie`):** A failed metadata read following a liveness probe
  must never be reported as `process_state.value = "zombie"`; zombie state
  requires direct kernel state observation (`p_stat == SZOMB` on macOS, state
  `Z` on Linux) bracketed to the target incarnation.

## Host scope, clone behavior and disclosure policy

`process.host_scope` carries `{namespace, value, derivation, stability,
clone_behavior}` and an optional report-local `identifier_ref`. It is a
**descriptor supplied across an explicit caller boundary, not authentication**.

- **Per-observation caller trust and stability:** Consumer policy supplies
  explicit boolean flags `baseline_host_scope_trusted` and
  `candidate_host_scope_trusted` with **no defaults** (omitting either flag or
  supplying a non-boolean is a policy refusal, never defaulted to `true`).
  Strict incarnation `match` requires both `baseline_host_scope_trusted == true`
  and `candidate_host_scope_trusted == true`, and requires both
  `host_scope.stability` values to be in `{boot, installation, enrollment,
  device}`. Untrusted scope or `stability` in `{unknown, process}` returns
  `insufficient:unestablished-host-scope`. When both scopes are trusted and
  established:
  - differing `(namespace, value, derivation)` returns
    `mismatch:host-scope-mismatch`;
  - equal `(namespace, value, derivation)` with differing `(stability,
    clone_behavior)` returns `insufficient:inconsistent-host-scope-metadata`.
- **Clone, snapshot restore and reinstall behavior:** `clone_behavior` in
  `{copyable, unknown}` discloses that VM cloning, golden-image copying or live
  snapshot restore can duplicate `host_scope` across distinct instances; a
  restored live snapshot can also duplicate `boot_id` and `start_identity` in
  both branches until reboot. Ordinary `match` **does not exclude** clones unless
  consumer policy sets `allowed_clone_behaviors` to a non-copyable subset such
  as `["regenerated", "device-bound"]`; when either observation's
  `host_scope.clone_behavior` is outside `allowed_clone_behaviors`, comparison
  returns `insufficient:disallowed-clone-behavior`. Reinstall or provisioning
  reset under `installation` or `regenerated` produces a distinct `value` and
  compares as `mismatch:host-scope-mismatch`.
- **Default probe policy and low-entropy disclosure:**
  - Default collection probes incarnation facts (`boot_id`,
    `pid_numbering_namespace`, `target_pid_namespace`, `pid`, `process_state`,
    `start_identity`, plus `observer.time_namespace` and
    `observer.user_namespace`) and
    `execution_context.security_principal` (`status: "observed"` by default
    unless the caller explicitly sets `probe_security_principal = false`, in
    which case `security_principal` records `status: "not-probed"`).
  - The four digest facts (`namespace_context_digest`, `executable_digest`,
    `arguments_digest`, `security_context_digest`) default to `status:
    "not-probed"` unless explicitly opted in by the caller.
  - Raw `argv` vectors, `environ` entries, filesystem paths, working directories,
    hardware serials and unscoped native machine identifiers are excluded from
    portable payloads.
- **No confidentiality from principals or unsalted digests:** Numeric POSIX
  UID/GID strings and domain-separated SHA-256 context digests provide
  deterministic equality and integrity binding only. Low-entropy inputs (UIDs,
  short command lines, predictable flags, or standard binary images) remain
  guessable offline. Callers must not export `security_principal` or
  `arguments_digest` across untrusted boundaries when UIDs are sensitive or
  command-line arguments could carry secrets, and applications must not pass
  secrets via `argv`.

## Pinned execution-context derivations and golden preimages

Every `execution_context` fact with `status == "observed"` **must** carry a
`derivation` `{id, revision}` pinned in `profile.json` `context_derivations` for
the observation's `observer.platform_family` (non-`observed` facts forbid
`derivation`). `u32_be(n)` denotes the 4-byte unsigned big-endian encoding of
`n`. All `st_dev` device numbers in Linux namespace identifiers use canonical
kernel `major:minor` unsigned decimal pairs (`unix.Major(dev)` / `libc::major(dev)`
and `unix.Minor(dev)` / `libc::minor(dev)`) so `glibc`, `musl` and pure-Go
`CGO_ENABLED=0` collectors emit byte-identical strings.

### 1. `security_principal`

Every credential component (`<u32>`) in both derivations is a canonical unsigned
decimal integer strictly within `0..=4294967295` (`u32`, no leading zeros;
values above `4294967295` are rejected by semantic validation).

- **`urn:3leaps:identity-evidence:principal-derivation:macos-kinfo-ucred:v0`** (`macos`):
  - Source: `sysctl("kern.proc.pid", pid)` `kinfo_proc.kp_eproc` (`p_cred.p_ruid`,
    `e_ucred.cr_uid`, `p_cred.p_rgid`, primary effective GID `e_ucred.cr_groups[0]`;
    if `cr_ngroups < 1`, record `status: "error"`).
  - Principal object:
    `namespace = "urn:3leaps:posix-credentials:macos-kinfo:v0"`,
    `value = "ruid=<u32>;euid=<u32>;rgid=<u32>;egid=<u32>"`.
  - Example: `ruid=501;euid=501;rgid=20;egid=20`.
- **`urn:3leaps:identity-evidence:principal-derivation:linux-procfs-status-uidgid:v0`** (`linux`):
  - Source: `<pid_dir_fd>/status` `Uid:` (4 tab-separated `u32`: real, effective,
    saved-set, fs) and `Gid:` (4 tab-separated `u32`: real, effective, saved-set,
    fs). Missing or malformed lines map to `status: "error"`; `EACCES`/`EPERM`
    maps to `status: "inaccessible"`.
  - **Observer user-namespace scoping and unmapped/overflow ID fail-closed rule:**
    Linux `task_state` renders `Uid:` and `Gid:` relative to the **reading
    observer's** user namespace (`seq_user_ns(m)`) via `from_kuid_munged` /
    `from_kgid_munged`, substituting `/proc/sys/kernel/overflowuid` /
    `overflowgid` (default `65534`) whenever a kernel ID is unmapped in that
    user namespace. Let `<userns_scope>` be `observer.user_namespace.value`
    (`userns:<major>:<minor>:<ino>`) when `observer.user_namespace.status ==
    "observed"`, or `userns:initial` when `observer.user_namespace.status ==
    "not-present"` (`CONFIG_USER_NS=n` kernel with a single initial user
    namespace):
    - When `observer.user_namespace.status == "observed"`, the collector reads
      `<self_dir_fd>/uid_map`, `<self_dir_fd>/gid_map`,
      `<proc_fd>/sys/kernel/overflowuid`, and `<proc_fd>/sys/kernel/overflowgid`
      (via Class B `openat2`). A rendered UID `u` (resp. GID `g`) is
      **unambiguously mapped** only when `u != 4294967295` (`(uid_t)-1`
      `INVALID_UID`) and either the observer's `uid_map` (resp. `gid_map`) covers
      the full valid kernel range (`0 0 4294967295`), **or** `u` falls inside a
      mapped `[first, first + count)` interior extent of `<self_dir_fd>/uid_map`
      **and** `u != overflowuid` (resp. `g != overflowgid`). If any of the 4 UIDs
      or 4 GIDs fails this check, the collector **must not** emit `observed`
      (which would equate unmapped principals or collide with mapped `nobody`);
      it records `status: "unknown", reason: "unmapped-or-overflow-id"`.
  - Principal object:
    `namespace = "urn:3leaps:posix-credentials:linux-procfs-status:v0:<userns_scope>"`,
    `value = "ruid=<u32>;euid=<u32>;suid=<u32>;fsuid=<u32>;rgid=<u32>;egid=<u32>;sgid=<u32>;fsgid=<u32>"`.
  - Example:
    `namespace = "urn:3leaps:posix-credentials:linux-procfs-status:v0:userns:0:4:4026531837"`,
    `value = "ruid=1000;euid=1000;suid=1000;fsuid=1000;rgid=1000;egid=1000;sgid=1000;fsgid=1000"`.

### 2. `arguments_digest`

- **`urn:3leaps:identity-evidence:context-derivation:argv-bytes:v0`** (`linux`, `macos`):
  - Preimage:
    `"3leaps.native-process-observation.arguments.v0\0" || u32_be(argc) || concat_{i=0..argc-1}(u32_be(len(arg_i)) || arg_i)`
  - Recovery and failure mapping (maximum raw read buffer **262,144 bytes**;
    exceeding 262,144 bytes refuses with `status: "error", reason:
    "arguments-exceed-cap"`):
    - **Linux:** Read `<pid_dir_fd>/cmdline` (up to 262,145 bytes). Empty file
      (0 bytes, e.g. kernel thread or zombie whose mm was released) maps to
      `status: "not-present", reason: "empty-cmdline"`. Otherwise, if the byte
      slice ends with `0x00`, strip exactly one trailing `0x00` and split on
      `0x00` into raw byte elements `arg_0 .. arg_{argc-1}` (preserving any
      empty elements or non-UTF-8 bytes verbatim; if a process rewrote its
      `argv` stack without NUL terminators, the non-empty buffer without trailing
      NUL yields the single element `[raw_bytes]`). `EACCES`/`EPERM` maps to
      `inaccessible`.
    - **macOS:** Query `sysctl([CTL_KERN, KERN_PROCARGS2, pid])` (up to 262,144
      bytes). Parse leading `i32` `argc` (`argc >= 0`), skip the exec-path NUL
      string and padding NULs to the start of `argv[0]`, then extract exactly
      `argc` NUL-terminated byte slices. `EPERM`/`EACCES` (which macOS enforces
      across UIDs and hardened runtimes) maps to `inaccessible`; truncated or
      malformed buffers map to `error`.
  - Golden vector: `argv = [b"/usr/bin/sleep", b"300"]` (`argc = 2`) →
    `sha256:3846a65c6cdc97a8388e79fad49c2837d9e0abacec2bdeef7f4b54d361b970f1`.

### 3. `executable_digest`

- **`urn:3leaps:identity-evidence:context-derivation:executable-file-bytes:v0`** (`linux`, `macos`):
  - Preimage:
    `"3leaps.native-process-observation.executable-image.v0\0" || image_bytes`
  - Source and size bound: Stream regular-file content up to **67,108,864 bytes**
    (64 MiB; files exceeding 64 MiB map to `status: "error", reason:
    "executable-exceeds-size-cap"`).
    - **Linux:** Open `<pid_dir_fd>/exe` via `openat(pid_dir_fd, "exe", O_RDONLY |
      O_CLOEXEC)` (intentionally following the kernel magic link to the backing
      regular file `inode`, never hashing a pathname string), verify `S_ISREG` via
      `fstat`, and stream bytes. `EACCES`/`EPERM` maps to `inaccessible`; `ENOENT`
      (for example kernel thread without backing executable) maps to
      `not-present`.
    - **macOS:** Resolve executable path via `proc_pidpath(pid)` (or
      `KERN_PROCARGS2` exec path), open with `O_RDONLY | O_CLOEXEC`, verify
      `S_ISREG`, and stream bytes within the $S_0 = S_1$ bracket.
  - Residual: Retains `no-running-memory-attestation` (an on-disk or `procfs`
    file read does not attest mapped pages after modification or `memfd` seal
    absence) and `macos-path-swap-window` (macOS resolves a pathname via
    `proc_pidpath` before `open`, so an on-disk path replacement between
    resolution and open is not inode-locked as Linux `<pid_dir_fd>/exe` is).
  - Golden vector: `image_bytes = b"\x7fELF-synthetic-sleep-image-v0\n"` (30
    bytes) →
    `sha256:58b9da5ab4aede636708b24b04ccfa6946746ab58058ec2e15ebda26b0a6d909`.

### 4. `namespace_context_digest`

- **`urn:3leaps:identity-evidence:context-derivation:linux-ns-inum-set:v0`** (`linux` only; `macos` is `unsupported`):
  - Member set (exact 9-element Linux namespace tuple in strict ASCII order):
    `("cgroup", "ipc", "mnt", "net", "pid", "time", "time_for_children", "user",
    "uts")`. Note: `pid_for_children` is intentionally excluded because it reflects
    future child placement rather than the process's own active namespace set.
  - Discovery and encoding: Open `pid_ns_fd = openat2(pid_dir_fd, "ns", ...)`
    under Class B below, then for each `kind` in the 9-element list open the
    single-component magic link via `ns_entry_fd = openat(pid_ns_fd, kind,
    O_RDONLY | O_CLOEXEC)` (following the magic link to the target `nsfs` inode —
    **never** `AT_SYMLINK_NOFOLLOW`, which stats the `procfs` symlink inode
    instead of the namespace), verify `fstatfs(ns_entry_fd).f_type == NSFS_MAGIC`
    (`0x6e736673`), read `fstat(ns_entry_fd)`, and encode
    `ns_id = "ns:<major>:<minor>:<ino>"` where `<major> = major(st.st_dev)`
    (`0..=4294967295`), `<minor> = minor(st.st_dev)` (`0..=4294967295`), and
    `<ino> = st.st_ino` (`0..=18446744073709551615`) as canonical unsigned
    decimals.
  - Failure mapping: No partial subset is ever hashed. If any of the 9 entries
    fails with `EACCES`/`EPERM`, record `inaccessible`; if any entry is absent on
    an older kernel (`ENOENT`, e.g. pre-5.6 without `ns/time`), record
    `unsupported`; otherwise on unexpected OS error record `error`.
  - Preimage:
    `"3leaps.native-process-observation.namespaces.v0\0" || u32_be(9) || concat_{(kind, ns_id)}(u32_be(len(kind)) || kind || u32_be(len(ns_id)) || ns_id)`
  - Golden vector:
    `[("cgroup","ns:0:4:4026531835"), ("ipc","ns:0:4:4026531839"), ("mnt","ns:0:4:4026531840"), ("net","ns:0:4:4026531992"), ("pid","ns:0:4:4026531836"), ("time","ns:0:4:4026531834"), ("time_for_children","ns:0:4:4026531834"), ("user","ns:0:4:4026531837"), ("uts","ns:0:4:4026531838")]`
    → `sha256:c74496ded4d8d5b0f0e11bfed4c3643e2496880b26502f885880355ec117bded`.

### 5. `security_context_digest`

- **`urn:3leaps:identity-evidence:context-derivation:linux-procfs-status-secctx:v0`** (`linux`; `macos` security-context derivation is `deferred` in this draft and must record `unsupported` or `not-probed`):
  - Source: `<pid_dir_fd>/status` fields `Groups:`, `NoNewPrivs:`, `Seccomp:`,
    `CapInh:`, `CapPrm:`, `CapEff:`, `CapBnd:`, `CapAmb:`. If any of these 8
    fields is missing (for example `CONFIG_SECCOMP` disabled), record
    `unsupported` (or `error` on malformed syntax; `inaccessible` on
    `EACCES`/`EPERM`). Because `Groups:` is rendered in the observer's user
    namespace via `from_kgid_munged`, every supplementary GID must pass the same
    unambiguous GID mapping check as `security_principal` (`0..=4294967295`, not
    `4294967295` `INVALID_GID`, and unambiguously mapped in `<self_dir_fd>/gid_map`
    when `observer.user_namespace` is `observed`); any unmapped/overflow GID
    maps to `status: "unknown", reason: "unmapped-or-overflow-gid"`. LSM labels
    (`/proc/<pid>/attr/current`, SELinux, AppArmor) are outside this derivation
    and require a separate LSM profile.
  - Canonical ASCII `canonical_context_bytes` (with `<userns_scope>` equal to
    `observer.user_namespace.value` when `observed` or `userns:initial` when
    `not-present`, `seccomp` and each GID in `0..=4294967295`):
    `"observer_user_ns=<userns_scope>;groups=<csv_sorted_dedup_u32>;no_new_privs=<0|1>;seccomp=<u32>;cap_inh=<16_lower_hex>;cap_prm=<16_lower_hex>;cap_eff=<16_lower_hex>;cap_bnd=<16_lower_hex>;cap_amb=<16_lower_hex>"`
    (when `Groups:` is empty, `<csv_sorted_dedup_u32>` is the empty string `""`;
    supplementary GIDs are sorted numerically ascending and deduplicated).
  - Preimage (`scheme` is the exact UTF-8 derivation ID
    `"urn:3leaps:identity-evidence:context-derivation:linux-procfs-status-secctx:v0"`):
    `"3leaps.native-process-observation.security-context.v0\0" || u32_be(len(scheme)) || scheme || "\0" || u32_be(len(canonical_context_bytes)) || canonical_context_bytes`
  - Golden vector:
    `canonical_context_bytes = b"observer_user_ns=userns:0:4:4026531837;groups=27,1000;no_new_privs=1;seccomp=2;cap_inh=0000000000000000;cap_prm=0000000000000000;cap_eff=0000000000000000;cap_bnd=000001ffffffffff;cap_amb=0000000000000000"`
    → `sha256:954107c2e47b6b4406093ee4e6cf14a66ee9c7350d6550f606b9995104d44976`.

## Numeric PID bounds, Linux descriptor/namespace defenses and start-identity sub-profiles

`process.pid` is a canonical positive decimal string in `1..=2147483647`
(`i32::MAX`, no leading zeros). Signalling sentinels (`0`, negative values, and
values above `2147483647`) are rejected by schema and collector input checks.

### Linux verified `procfs` root, mount-crossing defense, thread guard and time namespace

On Linux, numeric PIDs are relative to a PID namespace (`pidfd_open(pid, 0)`
resolves `pid` in the caller's active PID namespace, whereas `/proc/<pid>`
resolves `<pid>` in the `procfs` superblock's PID namespace), `/proc/<tid>`
accepts thread IDs that are not thread-group leaders, a mount administrator can
overmount `/proc/<pid>` or `/proc/self/status`, and `/proc/<pid>/stat` field 22
(`starttime`) is adjusted by the **reading observer's** time-namespace boottime
offset (`timens_add_boottime_ns`, Linux 5.6+).

Before emitting a Linux observation or opening a `pidfd`, a collector must
enforce all seven descriptor-anchored checks using three distinct resolution
classes:

1. **Verified `/proc` root (`proc_fd`):** Open `/proc` with `O_RDONLY |
   O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW` and verify `fstatfs(proc_fd).f_type ==
   PROC_SUPER_MAGIC` (`0x9fa0`).
2. **Three explicit descriptor-resolution classes and `ENOSYS` refusal:**
   - **Class A — Self-PID symlink discovery (`readlinkat` only):** `/proc/self`
     is a kernel symlink, so collectors **never** pass `"self"` or `"self/..."`
     to `openat2`, `openat`, or `fstatat`. Instead, call `readlinkat(proc_fd,
     "self", ...)` directly on `proc_fd` and verify the link text equals
     canonical decimal `getpid()`.
   - **Class B — `procfs` directories and ordinary `procfs` regular files
     (`openat2` strict mount/symlink confinement):** Open every `procfs`
     directory and regular file (`self_dir_fd` at `"<getpid()>"`, `self_ns_fd` at
     `"ns"` from `self_dir_fd`, `pid_dir_fd` at `"<pid>"`, `pid_ns_fd` at `"ns"`
     from `pid_dir_fd`, `"sys/kernel/random/boot_id"`,
     `"sys/kernel/overflowuid"`, `"sys/kernel/overflowgid"`, and `"status"`,
     `"stat"`, `"cmdline"`, `"uid_map"`, `"gid_map"` inside `self_dir_fd` or
     `pid_dir_fd`) using `openat2` with `resolve = RESOLVE_BENEATH |
     RESOLVE_NO_XDEV | RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`. Any `EXDEV`
     or `ELOOP` (descendant mount, same-superblock `/proc` bind mount, or
     symlink substitution) refuses the observation (`error` / collector refusal).
     **No `st_dev` fallback on `ENOSYS`:** because `st_dev` + `PROC_SUPER_MAGIC`
     cannot detect a bind mount of another node from the same `procfs`
     superblock (`mount --bind /proc/<other> /proc/<pid>`), a collector whose
     kernel returns `ENOSYS` for `openat2` (pre-5.6 Linux) **must refuse**
     (`unsupported`, `reason: "openat2-unavailable"`) rather than falling back to
     unconfined `openat`.
   - **Class C — Intentional single-component kernel magic links (`exe` and
     entries in `self_ns_fd` / `pid_ns_fd`):** In `openat2(2)`,
     `RESOLVE_NO_SYMLINKS` implies `RESOLVE_NO_MAGICLINKS`, `RESOLVE_BENEATH`
     also disables magic-link resolution, and magic-link targets (`nsfs` or
     backing executable filesystems) cross out of `procfs`. Therefore magic
     links are **never** opened with `openat2` and **never** walked via
     multi-component paths. Starting from the Class-B-verified `self_ns_fd`,
     `pid_ns_fd`, or `pid_dir_fd`, open only the single-component entry name
     with `openat(..., O_RDONLY | O_CLOEXEC)`: verify `fstatfs(fd).f_type ==
     NSFS_MAGIC` (`0x6e736673`) on namespace fds and `S_ISREG(fstat(fd).st_mode)`
     on `"exe"`. Residual `magic-link-leaf-not-mount-confined` remains disclosed
     because plain `openat` on the final magic-link leaf intentionally crosses
     mounts.
3. **Descriptor-anchored `NSpid` check on `self_dir_fd`:** Open `"status"` from
   `self_dir_fd` via Class B (never by path `"/proc/self/status"`) and require
   an explicit `NSpid:` line containing **exactly one** decimal token equal to
   `getpid()` (missing `NSpid:` or multi-token nested `NSpid:` refuses).
4. **Numbering, observer time, and observer user namespaces from `self_ns_fd`:**
   From Class-B-verified `self_ns_fd`, open `"pid"` via Class C and encode
   `pidns:<major>:<minor>:<ino>` as `pid_numbering_namespace`. Open `"time"` via
   Class C and encode `timens:<major>:<minor>:<ino>` as `observer.time_namespace`
   (if `"time"` fails with `ENOENT` on a kernel without time namespaces, record
   `observer.time_namespace = {status: "not-present", reason:
   "kernel-without-timens"}`). Open `"user"` via Class C and encode
   `userns:<major>:<minor>:<ino>` as `observer.user_namespace` (if `"user"` fails
   with `ENOENT` on a `CONFIG_USER_NS=n` kernel, record
   `observer.user_namespace = {status: "not-present", reason:
   "kernel-without-userns"}`).
5. **Boot ID newline stripping:** Read `"sys/kernel/random/boot_id"` relative to
   `proc_fd` via Class B, strip trailing `'\n'`, and validate lowercase UUID
   grammar.
6. **Thread-group leader guard (`Tgid == pid`):** Open `pid_dir_fd` for `"<pid>"`
   relative to `proc_fd` via Class B, read `"status"` from `pid_dir_fd` via
   Class B, parse `Tgid:`, and verify `Tgid == pid`. If `Tgid != pid` (`pid`
   names a non-leader thread TID), record `process_state = {status:
   "not-present", reason: "tid-not-thread-group-leader"}` and `start_identity =
   {status: "not-present", ...}` rather than recording a thread's start tick as
   a process incarnation.
7. **Safe `/proc/<pid>/stat` parsing and target PID namespace:** Read `"stat"`
   from `pid_dir_fd` via Class B, locate the **last** `')'` byte (`strrchr` /
   `bytes.LastIndexByte(')')`) closing `(comm)` (because `comm` may contain
   spaces, parentheses and newlines), require `') '` after it, split the
   remaining ASCII fields on single spaces, extract state (field 3, index 0
   after `') '`) and `starttime` (field 22, 0-based index 19 after `') '`), and
   open `"pid"` from Class-B-verified `pid_ns_fd` via Class C for
   `target_pid_namespace` (`pidns:<major>:<minor>:<ino>`; `EACCES`/`EPERM` under
   `ptrace_may_access` records `status: "inaccessible"` while keeping
   `pid_numbering_namespace` observed).

### Start-identity sub-profiles

| Sub-profile `id`                                                       | Platform  | `boot_id` grammar & source                                                  | Namespace rules (`pid_*`, `observer.time_namespace`, `observer.user_namespace`)                                                                                                                               | `start_identity.value` grammar & range                                        | Clock behavior & collision residual                                                                                 |
| ---------------------------------------------------------------------- | --------- | --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `urn:3leaps:identity-evidence:start-identity:linux-procfs-stat:v0`     | `linux`   | Lowercase UUID from `<proc_fd>/sys/kernel/random/boot_id` (newline-trimmed) | `pid_numbering_namespace` & `target_pid_namespace` required `observed` (`pidns:<maj>:<min>:<ino>`); `observer.time_namespace` (`timens:...`) and `observer.user_namespace` (`userns:...`) `observed` or `not-present` | Unsigned 64-bit decimal ticks (`0..=18446744073709551615`), `stat` field 22   | Monotonic boot ticks (`USER_HZ`, adjusted by observer `timens`); residual `sub-tick-pid-reuse-window`                |
| `urn:3leaps:identity-evidence:start-identity:macos-kinfo-starttime:v0` | `macos`   | Lowercase UUID from `sysctl("kern.bootsessionuuid")` (NUL/newline-trimmed)  | `pid_numbering_namespace`, `target_pid_namespace`, `observer.time_namespace`, and `observer.user_namespace` all required `unsupported`                                                                        | `<sec>.<usec>` (`sec` in `0..=9223372036854775807`, 6-digit `000000..999999`) | XNU `p_starttime` (`microtime` wall clock at `fork`); residual `wall-clock-step-and-microsecond-reuse-window`       |
| `urn:3leaps:identity-evidence:start-identity:windows-filetime:v0`      | `windows` | Reserved (not qualified in initial draft)                                   | Reserved `unsupported`                                                                                                                                                                                        | Reserved unsigned 64-bit 100 ns `FILETIME` decimal                            | Reserved; verifiers must return `insufficient:unsupported-start-identity-profile` until a Windows cell is qualified |

On macOS, `status: "unsupported"` on PID/time namespace fields applies to
`macos-kinfo-starttime:v0` only; it is **never a wildcard** that matches a Linux
observation or excuses missing Linux namespace facts.

### Pre/post incarnation bracketing and non-atomicity

Collectors must bracket every observation snapshot and every continuity-handle
registration:

1. Capture pre-snapshot $S_0 = (\text{numbering\_view}_0, \text{time\_ns}_0,
   \text{user\_ns}_0, \text{boot\_id}_0, \text{target\_pid\_ns}_0, \text{tgid}_0,
   \text{pid}, \text{start\_identity}_0, \text{state}_0)$.
2. Collect requested `execution_context` facts and, when requested, open the
   read-only continuity handle (`pidfd_open(pid, 0)` on Linux; `kqueue`
   `EVFILT_PROC` `NOTE_EXIT` registration on macOS — omit `NOTE_EXITSTATUS` so
   status-collection restrictions never masquerade as process exit).
3. Capture post-snapshot $S_1$ over the same incarnation tuple (and re-read
   `security_principal` when collected from a separate syscall/file).
4. Emit `observer.bracketed = true` only when $S_0 = S_1$ (for `state`, live→zombie
   transition during bracketing discards and retries once or refuses as
   unbracketed/`error` so mixed context is never emitted).

Residuals even when $S_0 = S_1$: in-place context changes within the same
incarnation (`setuid`, `prctl`, `unshare`) between intermediate reads and
post-observation TOCTOU between $S_1$ and a consumer's downstream action
(`post-observation-toctou-window`).

## Standalone live freshness gate (`admit_live_observation`)

`compare_native_observations` is a **pure snapshot comparison** (usable for both
live reobservation and historical audit without a wall-clock input). Live
reobservation admission and `local-process-session:v0` projection additionally
enforce a standalone freshness gate against the consumer's trusted UTC clock
`now`. On refusal, the gate returns/raises a typed refusal carrying the exact
code below in strict top-to-bottom order:

1. Validate `now` against `^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$`
   and calendar validity; if `now` is missing, `null`, non-string or malformed,
   refuse with `trusted-clock-unavailable`.
2. Require `payload.profile == accepted_profile_ref` (refusing
   `unsupported-evidence-profile` otherwise), then validate `payload.observed_at`
   against the same strict UTC instant grammar/calendar check (refusing
   `invalid-observed-at` when missing, non-string or malformed).
3. When bound to a report (`report` supplied, optionally with `input_reports`),
   enforce `necessary_conditions(report, input_reports)` and resolve the unique
   `os-observation` evidence descriptor matching `format_profile` or canonical
   payload `digest` (refusing `observation-evidence-descriptor-missing` when not
   unique), requiring `descriptor.format_profile == accepted_profile_ref`,
   `descriptor.digest == SHA256(canonical payload)`,
   `descriptor.method == "os-observation"`,
   `descriptor.subject_ref == payload.process.subject_ref`, and
   `descriptor.collected_at == payload.observed_at` (refusing
   `observation-evidence-descriptor-mismatch` otherwise; `input_reports`
   provenance prevents rewriting `collected_at` on imported observations).
4. Compute `age_seconds = (now - observed_at)` in whole seconds:
   - if `age_seconds < -max_future_skew_seconds` (`max_future_skew_seconds = 0`,
     so `observed_at > now`), refuse with `future-process-observation`;
   - if `age_seconds > max_observation_age_seconds` (`max_observation_age_seconds = 5`,
     so `(now - observed_at) > 5`), refuse with `stale-process-observation`;
   - inclusive boundaries `age_seconds == 0` and `age_seconds == 5` are accepted.

## Deterministic comparison rules (`compare_native_observations`)

### Policy object validation (no defaults)

`policy` must be a JSON object/mapping containing **all five** keys with exact
types (no defaults; any missing key, `null`, or wrong type — including integer `1`
for a boolean — returns `insufficient:invalid-evaluation-policy`):

- `baseline_host_scope_trusted`: `boolean`
- `candidate_host_scope_trusted`: `boolean`
- `require_live`: `boolean`
- `required_context_facts`: array/sequence of valid `execution_context` fact
  names (duplicates allowed, evaluated in canonical schema order below; unknown
  fact name returns `insufficient:invalid-evaluation-policy`)
- `detect_observed_context_drift`: `boolean`
- Optional sixth key `allowed_clone_behaviors`: when present, must be a
  non-empty array/sequence of valid `clone_behavior` strings (`unknown`,
  `copyable`, `regenerated`, `device-bound`); invalid type/value or unknown
  extra policy keys return `insufficient:invalid-evaluation-policy`.

### Canonical evaluation order and complete result-code table

The table below is the **single normative multi-fault authority** and is
evaluated strictly **row-by-row from top to bottom**: within every row that
inspects both observations (`Either ...`), `baseline` and `candidate` are **both**
checked for that row's condition before evaluation advances to the next row.
Grammar and cross-field consistency rows always precede value-equality rows.
Canonical `execution_context` fact order is fixed to schema property order:
`("security_principal", "namespace_context_digest", "executable_digest",
"arguments_digest", "security_context_digest")`.

| Step | Condition (evaluated strictly row-by-row top-to-bottom across both sides) | Exact returned result code |
| ---- | ------------------------------------------------------------------------- | -------------------------- |
| 1a | `policy` fails strict schema/type check above | `insufficient:invalid-evaluation-policy` |
| 1b | Either `obs.profile != accepted_profile_ref` | `insufficient:unsupported-evidence-profile` |
| 1c | Either `obs.observer.bracketed != true` | `insufficient:unbracketed-observation` |
| 1d | Either `start_identity.profile` is unknown, `reserved` (`windows-filetime:v0`), or has wrong `revision` | `insufficient:unsupported-start-identity-profile` |
| 1e | Either `obs.observer.platform_family != subprofile.platform_family` | `insufficient:inconsistent-observation` |
| 2a | `baseline_host_scope_trusted != true` or `candidate_host_scope_trusted != true` or either `host_scope.stability` not in `{boot, installation, enrollment, device}` | `insufficient:unestablished-host-scope` |
| 2b | Either `(namespace, value, derivation)` differs between `baseline.process.host_scope` and `candidate.process.host_scope` | `mismatch:host-scope-mismatch` |
| 2c | `stability` or `clone_behavior` differs between `baseline.process.host_scope` and `candidate.process.host_scope` | `insufficient:inconsistent-host-scope-metadata` |
| 2d | `policy.allowed_clone_behaviors` is present and either `host_scope.clone_behavior` is not in `allowed_clone_behaviors` | `insufficient:disallowed-clone-behavior` |
| 3a | Either `boot_id.status != "observed"` | `insufficient:missing-boot-id` |
| 3b | Either `boot_id.value` fails `subprofile.boot_id_pattern` | `insufficient:invalid-boot-id` |
| 3c | `baseline.process.boot_id.value != candidate.process.boot_id.value` | `mismatch:boot-id-mismatch` |
| 4a | `baseline.process.start_identity.profile != candidate.process.start_identity.profile` | `insufficient:incompatible-start-identity-profile` |
| 4b | Under `linux-procfs-stat:v0`: either `pid_numbering_namespace.status != "observed"` | `insufficient:missing-pid-numbering-namespace` |
| 4c | Under `linux-procfs-stat:v0`: either `pid_numbering_namespace.value` fails `pidns:<u32>:<u32>:<u64>` grammar/range | `insufficient:invalid-pid-namespace` |
| 4d | Under `linux-procfs-stat:v0`: `baseline.pid_numbering_namespace.value != candidate.pid_numbering_namespace.value` | `insufficient:pid-numbering-namespace-mismatch` |
| 4e | Under `linux-procfs-stat:v0`: either `observer.time_namespace.status` not in `{"observed", "not-present"}` | `insufficient:missing-observer-time-namespace` |
| 4f | Under `linux-procfs-stat:v0`: either `observer.time_namespace` is `observed` and fails `timens:<u32>:<u32>:<u64>` grammar/range | `insufficient:invalid-observer-time-namespace` |
| 4g | Under `linux-procfs-stat:v0`: `observer.time_namespace` `(status, value)` differs between `baseline` and `candidate` (`reason` strings on two `not-present` facts are ignored) | `insufficient:observer-time-namespace-mismatch` |
| 4h | Under `linux-procfs-stat:v0`: either `observer.user_namespace.status` not in `{"observed", "not-present"}` | `insufficient:missing-observer-user-namespace` |
| 4i | Under `linux-procfs-stat:v0`: either `observer.user_namespace` is `observed` and fails `userns:<u32>:<u32>:<u64>` grammar/range | `insufficient:invalid-observer-user-namespace` |
| 4j | Under `linux-procfs-stat:v0`: `observer.user_namespace` `(status, value)` differs between `baseline` and `candidate` (`reason` strings on two `not-present` facts are ignored) | `insufficient:observer-user-namespace-mismatch` |
| 4k | Under `macos-kinfo-starttime:v0`: either observation has `pid_numbering_namespace.status != "unsupported"`, `target_pid_namespace.status != "unsupported"`, `observer.time_namespace.status != "unsupported"`, or `observer.user_namespace.status != "unsupported"` | `insufficient:inconsistent-observation` |
| 5a | Either `pid` fails canonical decimal range `1..=2147483647` | `insufficient:invalid-pid-range` |
| 5b | `baseline.process.pid != candidate.process.pid` | `mismatch:pid-mismatch` |
| 5c | `baseline.process.process_state.status == "observed"` and `baseline.process.start_identity.status == "observed"`, while `candidate.process.process_state.status == "not-present"` or `candidate.process.start_identity.status == "not-present"` | `mismatch:process-exited` |
| 5d | Under `linux-procfs-stat:v0`: either `target_pid_namespace.status != "observed"` | `insufficient:missing-target-pid-namespace` |
| 5e | Under `linux-procfs-stat:v0`: either `target_pid_namespace.value` fails `pidns:<u32>:<u32>:<u64>` grammar/range | `insufficient:invalid-pid-namespace` |
| 5f | Under `linux-procfs-stat:v0`: `baseline.target_pid_namespace.value != candidate.target_pid_namespace.value` | `mismatch:target-pid-namespace-mismatch` |
| 6a | Either `start_identity.status != "observed"` | `insufficient:missing-start-identity` |
| 6b | Either `start_identity.value` fails `subprofile` grammar/range (`u64` on Linux; `<i64_nonneg>.<6digits>` on macOS) | `insufficient:invalid-start-identity` |
| 6c | `baseline.process.start_identity.value != candidate.process.start_identity.value` | `mismatch:start-identity-mismatch` |
| 7a | Either `process_state.status != "observed"` | `insufficient:missing-process-state` |
| 7b | `policy.require_live == true` and `candidate.process.process_state.value != "live"` | `mismatch:process-not-live` |
| 8a | For any `fact` in canonical order where either observation has `status == "observed"` with an unaccepted/deferred/wrong-platform `derivation` **or** (for `security_principal`) a `principal` `namespace`/`value` failing the derivation's pinned namespace/user-namespace scope, grammar, or `0..=4294967295` `u32` component bounds | `insufficient:unsupported-context-derivation-<fact>` |
| 8b | Pass 1 over `fact` in canonical order: if `fact` is in `required_context_facts` and either observation has `status != "observed"` | `insufficient:missing-context-<fact>` |
| 8c | Pass 2 over `fact` in canonical order: if `fact` is in `required_context_facts` (or `detect_observed_context_drift == true` and both are `observed`) and `baseline` vs `candidate` `derivation` differs | `insufficient:incompatible-context-derivation-<fact>` |
| 8d | Pass 3 over `fact` in canonical order: if `fact` is in `required_context_facts` (or `detect_observed_context_drift == true` and both are `observed`) and `principal`/`digest` differs | `mismatch:context-changed-<fact>` |
| 9 | All checks above pass | `match` |

Note on Step 8 multi-pass precedence: evaluating all missing required facts
(Pass 1, Step 8b) across canonical fact order before derivation compatibility
(Pass 2, Step 8c) and value inequality (Pass 3, Step 8d) makes comparison
completely independent of the caller's list ordering in `required_context_facts`.

## Composition and injective projection into `local-process-session:v0`

`local-process-session:v0` requires a flat string `process.host_scope`
(`maxLength: 512`), an observed `process.pid_namespace` string, and all five
`execution_context` fields unconditionally. Consumers must **not** substitute
this profile's `process` object directly, concatenate `<namespace>/<value>`
without framing, or invent sentinel strings, empty hashes or `null` values for
unavailable native fields.

Instead, an evaluator building a `local-process-session:v0` report imports the
observation report through `input_reports` (preserving collection time and full
structured `host_scope` / `identifiers[]` provenance) and runs a fail-closed
projection adapter (`project_to_local_process_session_process`) that succeeds
**only** when:

1. `admit_live_observation` succeeds against trusted consumer clock `now`
   (`0 <= now - observed_at <= 5`, plus report `collected_at == observed_at`
   when descriptor-bound);
2. `host_scope_trusted` is explicitly boolean `true` and `host_scope.stability`
   is in `{boot, installation, enrollment, device}` (and `host_scope.clone_behavior`
   is in `allowed_clone_behaviors` when caller policy specifies one);
3. `observer.bracketed` is `true`, `observer.platform_family` matches the
   accepted `start_identity` sub-profile, and `process_state` is `observed` as
   `"live"`;
4. `boot_id` and `start_identity` are both `observed` and valid under the
   accepted sub-profile, and both `observer.time_namespace` and
   `observer.user_namespace` satisfy the sub-profile rules (`observed` valid
   `timens:<maj>:<min>:<ino>` / `userns:<maj>:<min>:<ino>` or `not-present` on
   Linux);
5. `pid_numbering_namespace` and `target_pid_namespace` are both `observed`,
   valid (`pidns:<maj>:<min>:<ino>`), and **equal**, projecting
   `pid_namespace := target_pid_namespace.value`; and
6. all five `execution_context` facts have `status == "observed"` under accepted
   non-deferred `context_derivations` for `observer.platform_family` (including
   user-namespace-scoped `security_principal.namespace`, `0..=4294967295`
   credential bounds, and user-namespace-scoped `security_context_digest`).

### Clock-domain-scoped `start_identity` projection

Because Linux `/proc/<pid>/stat` `starttime` ticks are relative to
`observer.time_namespace`, `project_to_local_process_session_process` **never
drops** the observer's clock domain when emitting `process.start_identity`. On
Linux (`linux-procfs-stat:v0`), `process.start_identity` projects under the
scoped profile
`{"id": "urn:3leaps:identity-evidence:start-identity:linux-procfs-stat-timens-scoped:v0", "revision": <README_revision>}`
with injective `value`:

- `"<observer.time_namespace.value>;ticks=<start_identity.value>"` when
  `observer.time_namespace.status == "observed"` (for example
  `"timens:0:4:4026531834;ticks=987654321"`), or
- `"host-boottime;ticks=<start_identity.value>"` when
  `observer.time_namespace.status == "not-present"` (kernel without `timens`).

Any change to `observer.time_namespace` (including `observed` vs `not-present` or
differing `timens:` identifiers) changes the projected `process.start_identity`
and `process_digest`.

### Canonical `host_scope` tuple projection

To guarantee collision resistance across arbitrary `/` characters in `namespace`
or `value`, bind `derivation` (`{id, revision}`), `stability`, and
`clone_behavior`, and stay strictly within `local-process-session:v0`'s
`maxLength: 512` (even when `namespace` and `value` each use their full 512-byte
budget), `process.host_scope` projects to the fixed 114-character ASCII URN:

```
urn:3leaps:identity-evidence:host-scope:v0:sha256:<lowercase_hex>
```

where `<lowercase_hex>` is SHA-256 over the length-delimited, domain-separated
UTF-8 preimage:

```
"3leaps.native-process-observation.host-scope.v0\0"
  || u32_be(len(namespace))      || namespace
  || u32_be(len(value))          || value
  || u32_be(len(derivation.id))  || derivation.id
  || u32_be(len(derivation.revision)) || derivation.revision
  || u32_be(len(stability))      || stability
  || u32_be(len(clone_behavior)) || clone_behavior
```

Golden projection vector (`namespace = "example.test/installations"`,
`value = "host-a"`,
`derivation.id = "urn:example:identity:synthetic-identifier-derivation:v0"`,
`derivation.revision = "sha256:cf58fe77e63c6c91f67ef93302d6872790df131215e94a205c62b55ca84a56fe"`,
`stability = "installation"`, `clone_behavior = "copyable"`):

```
urn:3leaps:identity-evidence:host-scope:v0:sha256:8e3d1953b6494fc7c0399bf1487b90e57917b9a0651670dd83bc62a0af5d0a8d
```

When any predicate above is unmet (including macOS observations where PID
namespaces are `unsupported`, or default-policy observations where
`arguments_digest` is `not-probed`), projection refuses cleanly while the
standalone `native-process-observation:v0` report remains valid for consumers
whose policy accepts software-only process reobservation.
