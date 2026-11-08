// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration and owned-child qualification tests for `sysprims_proc::observation`.

use sysprims_evidence::ProfileRef;
use sysprims_proc::observation::{
    observe_process, open_continuity_handle, HostScope, ObservationOptions, MAX_OBSERVABLE_PID,
};

const ZERO_DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

fn sample_host_scope() -> HostScope {
    HostScope {
        identifier_ref: Some("id-host-local".to_string()),
        namespace: "urn:3leaps:identity-evidence:host-scope:test-fixture:v0".to_string(),
        value: "qualification-fixture-host".to_string(),
        derivation: ProfileRef::new(
            "urn:3leaps:identity-evidence:derivation:caller-supplied-host-scope:v0",
            ZERO_DIGEST,
        ),
        stability: "boot".to_string(),
        clone_behavior: "regenerated".to_string(),
    }
}

#[test]
fn refuses_out_of_range_pids_and_malformed_caller_inputs_before_syscalls() {
    let hs = sample_host_scope();
    let mut opts = ObservationOptions {
        observed_at: Some("2026-11-06T12:00:00Z".to_string()),
        ..ObservationOptions::default()
    };

    for bad_pid in [0u32, MAX_OBSERVABLE_PID + 1, u32::MAX] {
        let err = observe_process(bad_pid, &hs, &opts).expect_err("out-of-range PID must fail");
        assert_eq!(err.code, "invalid-pid-range");
        let herr =
            open_continuity_handle(bad_pid, &hs, &opts).expect_err("out-of-range PID must fail");
        assert_eq!(herr.code, "invalid-pid-range");
    }

    opts.subject_ref = "Invalid_Subject".to_string();
    let err = observe_process(42, &hs, &opts).expect_err("bad subject_ref");
    assert_eq!(err.code, "invalid-subject-ref");
    opts.subject_ref = "process-target".to_string();

    opts.observed_at = Some("2026-11-06T12:00:00.123Z".to_string());
    let err = observe_process(42, &hs, &opts).expect_err("fractional observed_at must fail");
    assert_eq!(err.code, "invalid-observed-at");
    opts.observed_at = Some("2026-11-06T12:00:00Z".to_string());

    let mut bad_hs = hs.clone();
    bad_hs.stability = "forever".to_string();
    let err = observe_process(42, &bad_hs, &opts).expect_err("invalid stability");
    assert_eq!(err.code, "invalid-host-scope");

    let mut bad_hs2 = hs;
    bad_hs2.identifier_ref = Some("UPPERCASE".to_string());
    let err = observe_process(42, &bad_hs2, &opts).expect_err("invalid identifier_ref");
    assert_eq!(err.code, "invalid-host-scope");
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn unsupported_platform_fails_closed_explicitly() {
    let hs = sample_host_scope();
    let opts = ObservationOptions {
        observed_at: Some("2026-11-06T12:00:00Z".to_string()),
        ..ObservationOptions::default()
    };
    let err = observe_process(4242, &hs, &opts).expect_err("unsupported OS must refuse");
    assert_eq!(err.status, "unsupported");
    assert_eq!(err.code, "windows-collector-not-qualified");

    let herr = open_continuity_handle(4242, &hs, &opts).expect_err("unsupported OS must refuse");
    assert_eq!(herr.status, "unsupported");
    assert_eq!(herr.code, "windows-collector-not-qualified");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_qualification {
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

    use serde_json::{json, Value};
    use sysprims_evidence::{
        admit_and_validate_report, admit_live_observation, compare_native_observations,
        derive_arguments_digest, derive_executable_digest,
        project_to_local_process_session_process, ComparisonOutcome, ComparisonPolicy,
    };
    use sysprims_proc::observation::{
        accepted_observation_profile, build_observation_report, observe_process,
        open_continuity_handle, CapabilityProbe, ObservationOptions, ReportMetadata,
    };

    use super::{sample_host_scope, HostScope, ZERO_DIGEST};

    struct OwnedChild {
        child: Child,
        reaped: bool,
    }

    impl OwnedChild {
        fn spawn_sleep() -> (Self, String) {
            let sleep_path = ["/bin/sleep", "/usr/bin/sleep"]
                .into_iter()
                .find(|p| Path::new(p).exists())
                .expect("sleep binary present at /bin/sleep or /usr/bin/sleep")
                .to_string();
            let child = Command::new(&sleep_path)
                .arg("30")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn owned sleep child");
            #[cfg(target_os = "linux")]
            {
                // glibc posix_spawn wakes the parent when FD_CLOEXEC closes in
                // begin_new_exec(), microseconds before create_elf_tables() sets
                // mm->arg_start/arg_end for /proc/<pid>/cmdline.
                let cmdline_path = format!("/proc/{}/cmdline", child.id());
                for _ in 0..50 {
                    if std::fs::read(&cmdline_path)
                        .map(|b| !b.is_empty())
                        .unwrap_or(false)
                    {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            (
                Self {
                    child,
                    reaped: false,
                },
                sleep_path,
            )
        }

        fn pid(&self) -> u32 {
            self.child.id()
        }

        fn kill_unreaped(&mut self) {
            let _ = self.child.kill();
        }

        fn reap(&mut self) {
            if !self.reaped {
                let _ = self.child.wait();
                self.reaped = true;
            }
        }
    }

    impl Drop for OwnedChild {
        fn drop(&mut self) {
            if !self.reaped {
                let _ = self.child.kill();
                let _ = self.child.wait();
                self.reaped = true;
            }
        }
    }

    fn platform_supported_context_facts() -> Vec<String> {
        if cfg!(target_os = "linux") {
            vec![
                "security_principal".to_string(),
                "namespace_context_digest".to_string(),
                "executable_digest".to_string(),
                "arguments_digest".to_string(),
                "security_context_digest".to_string(),
            ]
        } else {
            vec![
                "security_principal".to_string(),
                "executable_digest".to_string(),
                "arguments_digest".to_string(),
            ]
        }
    }

    /// Unprivileged OCI container runtimes (such as `docker run` without `--privileged`)
    /// mount `/proc/sys` as a read-only descendant bind mount over `/proc` (unlike bare-metal
    /// systemd VMs where only `/proc/sys/fs/binfmt_misc` is mounted outside `/proc/sys/kernel`).
    /// Under `native-process-observation:v0` Class B `openat2(RESOLVE_NO_XDEV)` confinement,
    /// any mountpoint along the collector's `/proc/sys/kernel` anchor paths must refuse closed
    /// with `procfs-confinement-violation`.
    fn linux_proc_sys_kernel_is_overmounted() -> bool {
        #[cfg(target_os = "linux")]
        {
            if let Ok(mi) = std::fs::read_to_string("/proc/self/mountinfo") {
                for line in mi.lines() {
                    let mut fields = line.split_whitespace();
                    if let Some(mount_point) = fields.nth(4) {
                        if matches!(
                            mount_point,
                            "/proc/sys"
                                | "/proc/sys/kernel"
                                | "/proc/sys/kernel/random"
                                | "/proc/sys/kernel/random/boot_id"
                                | "/proc/sys/kernel/overflowuid"
                                | "/proc/sys/kernel/overflowgid"
                        ) {
                            return true;
                        }
                    }
                }
            }
            false
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    fn verify_proc_sys_overmount_refuses_closed(
        pid: u32,
        hs: &HostScope,
        opts: &ObservationOptions,
    ) -> bool {
        if !linux_proc_sys_kernel_is_overmounted() {
            return false;
        }
        let obs_err = observe_process(pid, hs, opts)
            .expect_err("over-mounted /proc/sys must refuse under RESOLVE_NO_XDEV");
        assert_eq!(obs_err.status, "error");
        assert_eq!(obs_err.code, "procfs-confinement-violation");

        let handle_err = open_continuity_handle(pid, hs, opts)
            .expect_err("over-mounted /proc/sys must refuse continuity handle");
        assert_eq!(handle_err.status, "error");
        assert_eq!(handle_err.code, "procfs-confinement-violation");
        true
    }

    #[test]
    fn default_and_opt_in_disclosure_match_spawned_child_image_and_argv() {
        let (owned, sleep_path) = OwnedChild::spawn_sleep();
        let pid = owned.pid();
        let hs = sample_host_scope();

        // 1. Default disclosure: security_principal observed, all 4 digests not-probed.
        let default_opts = ObservationOptions {
            observed_at: Some("2026-11-06T12:00:00Z".to_string()),
            ..ObservationOptions::default()
        };
        if verify_proc_sys_overmount_refuses_closed(pid, &hs, &default_opts) {
            return;
        }
        let obs_default =
            observe_process(pid, &hs, &default_opts).expect("observe default owned child");
        assert_eq!(
            obs_default["process"]["process_state"],
            json!({"status": "observed", "value": "live"})
        );
        assert_eq!(
            obs_default["process"]["start_identity"]["status"],
            "observed"
        );
        assert_eq!(
            obs_default["process"]["execution_context"]["security_principal"]["status"],
            "observed"
        );
        for omitted in [
            "namespace_context_digest",
            "executable_digest",
            "arguments_digest",
            "security_context_digest",
        ] {
            assert_eq!(
                obs_default["process"]["execution_context"][omitted]["status"], "not-probed",
                "expected {omitted} to be not-probed by default"
            );
        }

        // 2. Full opt-in disclosure: executable_digest and arguments_digest match independent derivations.
        let full_opts = default_opts.with_all_context_facts();
        let obs_full = observe_process(pid, &hs, &full_opts).expect("observe full owned child");

        let expected_exe_digest =
            derive_executable_digest(&std::fs::read(&sleep_path).expect("read sleep binary"));
        assert_eq!(
            obs_full["process"]["execution_context"]["executable_digest"]["status"],
            "observed"
        );
        assert_eq!(
            obs_full["process"]["execution_context"]["executable_digest"]["digest"],
            expected_exe_digest
        );

        let expected_args_digest = derive_arguments_digest(&[sleep_path.as_bytes(), b"30"]);
        assert_eq!(
            obs_full["process"]["execution_context"]["arguments_digest"]["status"],
            "observed"
        );
        assert_eq!(
            obs_full["process"]["execution_context"]["arguments_digest"]["digest"],
            expected_args_digest
        );

        if cfg!(target_os = "linux") {
            assert_eq!(
                obs_full["process"]["target_pid_namespace"]["status"],
                "observed"
            );
            assert_eq!(
                obs_full["process"]["execution_context"]["namespace_context_digest"]["status"],
                "observed"
            );
            assert_eq!(
                obs_full["process"]["execution_context"]["security_context_digest"]["status"],
                "observed"
            );
        } else {
            assert_eq!(
                obs_full["process"]["target_pid_namespace"]["status"],
                "unsupported"
            );
            assert_eq!(
                obs_full["process"]["execution_context"]["namespace_context_digest"]["status"],
                "unsupported"
            );
            assert_eq!(
                obs_full["process"]["execution_context"]["security_context_digest"]["status"],
                "unsupported"
            );
        }
    }

    #[test]
    fn continuity_handle_tracks_live_zombie_and_reaped_transitions_and_pid_reuse() {
        let (mut owned, _sleep_path) = OwnedChild::spawn_sleep();
        let pid = owned.pid();
        let hs = sample_host_scope();
        let opts = ObservationOptions {
            observed_at: Some("2026-11-06T12:00:00Z".to_string()),
            ..ObservationOptions::default()
        }
        .with_all_context_facts();
        if verify_proc_sys_overmount_refuses_closed(pid, &hs, &opts) {
            return;
        }

        let handle = open_continuity_handle(pid, &hs, &opts).expect("open continuity handle");
        assert_eq!(handle.pid(), pid);
        assert!(!handle.poll_exited().expect("poll alive child"));
        assert!(!handle
            .wait_exit(Duration::from_millis(20))
            .expect("short wait on alive child"));

        let strict_policy = ComparisonPolicy {
            baseline_host_scope_trusted: true,
            candidate_host_scope_trusted: true,
            require_live: true,
            required_context_facts: platform_supported_context_facts(),
            detect_observed_context_drift: true,
            allowed_clone_behaviors: None,
        };

        let (_cand_live, outcome_live) = handle
            .reverify(&strict_policy, "2026-11-06T12:00:01Z")
            .expect("reverify alive child");
        assert_eq!(outcome_live, ComparisonOutcome::Match);

        // Terminate child WITHOUT reaping so it becomes a zombie (SZOMB / 'Z').
        owned.kill_unreaped();
        assert!(
            handle
                .wait_exit(Duration::from_secs(3))
                .expect("wait_exit after SIGKILL"),
            "continuity handle must wake upon child exit"
        );
        // Subsequent poll_exited must remain true idempotently.
        assert!(handle.poll_exited().expect("idempotent poll_exited"));

        // Wait briefly if needed until kernel finishes transitioning task state to zombie.
        let mut saw_zombie = false;
        for _ in 0..40 {
            let snap = handle
                .reobserve(Some("2026-11-06T12:00:02Z"))
                .expect("reobserve unreaped child");
            if snap["process"]["process_state"]["value"] == "zombie" {
                saw_zombie = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            saw_zombie,
            "unreaped terminated child must report zombie state"
        );

        let (_cand_zombie, outcome_zombie_live_req) = handle
            .reverify(&strict_policy, "2026-11-06T12:00:02Z")
            .expect("reverify zombie child");
        let relaxed_zombie_policy = ComparisonPolicy {
            baseline_host_scope_trusted: true,
            candidate_host_scope_trusted: true,
            require_live: false,
            required_context_facts: Vec::new(),
            detect_observed_context_drift: false,
            allowed_clone_behaviors: None,
        };
        let (_cand_zombie2, outcome_zombie_relaxed) = handle
            .reverify(&relaxed_zombie_policy, "2026-11-06T12:00:02Z")
            .expect("reverify zombie child with require_live=false");
        if cfg!(target_os = "linux") {
            // Linux do_exit() tears down task namespaces before entering EXIT_ZOMBIE,
            // so /proc/<pid>/ns/pid is not-present on zombies and Step 5d refuses closed.
            assert_eq!(
                outcome_zombie_live_req,
                ComparisonOutcome::Insufficient("missing-target-pid-namespace".to_string())
            );
            assert_eq!(
                outcome_zombie_relaxed,
                ComparisonOutcome::Insufficient("missing-target-pid-namespace".to_string())
            );
        } else {
            assert_eq!(
                outcome_zombie_live_req,
                ComparisonOutcome::Mismatch("process-not-live".to_string())
            );
            assert_eq!(outcome_zombie_relaxed, ComparisonOutcome::Match);
        }

        // Now reap the child so its PID disappears from the process table.
        owned.reap();
        let (_cand_reaped, outcome_reaped) = handle
            .reverify(&strict_policy, "2026-11-06T12:00:03Z")
            .expect("reverify reaped child");
        assert_eq!(
            outcome_reaped,
            ComparisonOutcome::Mismatch("process-exited".to_string())
        );

        // Opening a new continuity handle on the reaped PID must fail closed.
        let reopen_err =
            open_continuity_handle(pid, &hs, &opts).expect_err("reaped PID cannot open handle");
        assert_eq!(reopen_err.status, "not-present");

        // Spawn a second child to obtain a genuine distinct kernel start_identity,
        // and verify PID reuse detection when observed at the original PID slot.
        let (owned2, _) = OwnedChild::spawn_sleep();
        let mut reused_candidate = observe_process(owned2.pid(), &hs, &opts)
            .expect("observe second distinct child incarnation");
        // Ensure distinct start_identity even if spawned within the same tick/microsecond:
        if reused_candidate["process"]["start_identity"]["value"]
            == handle.baseline_observation()["process"]["start_identity"]["value"]
        {
            std::thread::sleep(Duration::from_millis(25));
            let (owned3, _) = OwnedChild::spawn_sleep();
            reused_candidate =
                observe_process(owned3.pid(), &hs, &opts).expect("observe third child");
        }
        reused_candidate["process"]["pid"] = Value::String(pid.to_string());
        let reuse_outcome = compare_native_observations(
            handle.baseline_observation(),
            &reused_candidate,
            &strict_policy,
            &accepted_observation_profile(),
        );
        assert_eq!(
            reuse_outcome,
            ComparisonOutcome::Mismatch("start-identity-mismatch".to_string())
        );
    }

    #[test]
    fn consumer_report_binding_freshness_gate_and_session_projection_exercise() {
        let (owned, _) = OwnedChild::spawn_sleep();
        let pid = owned.pid();
        let hs = sample_host_scope();
        let opts = ObservationOptions {
            observed_at: Some("2026-11-06T12:00:00Z".to_string()),
            ..ObservationOptions::default()
        }
        .with_all_context_facts();
        if verify_proc_sys_overmount_refuses_closed(pid, &hs, &opts) {
            return;
        }

        let obs = observe_process(pid, &hs, &opts).expect("observe child");
        let meta = ReportMetadata {
            exit_handle_capability: Some(CapabilityProbe {
                provider: if cfg!(target_os = "linux") {
                    "linux-pidfd".to_string()
                } else {
                    "macos-kqueue-proc".to_string()
                },
                status: "available".to_string(),
                reason: String::new(),
            }),
            ..ReportMetadata::default()
        };

        let admitted_report =
            build_observation_report(&obs, &meta).expect("build & admit observation report");
        let reparsed = admit_and_validate_report(&admitted_report.canonical_bytes, None)
            .expect("re-validate canonical report bytes");
        assert_eq!(reparsed.digest, admitted_report.digest);

        let accepted = accepted_observation_profile();
        admit_live_observation(
            &obs,
            Some("2026-11-06T12:00:05Z"),
            &accepted,
            Some(&admitted_report.value),
            None,
        )
        .expect("+5s boundary must pass freshness gate");

        let stale_err = admit_live_observation(
            &obs,
            Some("2026-11-06T12:00:06Z"),
            &accepted,
            Some(&admitted_report.value),
            None,
        )
        .expect_err("+6s must refuse as stale");
        assert_eq!(stale_err.code(), "stale-process-observation");

        let proj_res = project_to_local_process_session_process(
            &obs,
            true,
            Some("2026-11-06T12:00:01Z"),
            &accepted,
            Some(&admitted_report.value),
            None,
            None,
        );
        if cfg!(target_os = "linux") {
            let projected =
                proj_res.expect("Linux full-context observation must project to session process");
            let hs_urn = projected["host_scope"]
                .as_str()
                .expect("projected host_scope string");
            assert!(hs_urn.starts_with("urn:3leaps:identity-evidence:host-scope:v0:sha256:"));
            assert_eq!(
                projected["start_identity"]["profile"]["id"],
                sysprims_evidence::LINUX_PROCFS_STAT_TIMENS_SCOPED_PROFILE_ID
            );
            let start_val = projected["start_identity"]["value"]
                .as_str()
                .expect("projected timens-scoped start_identity");
            assert!(start_val.starts_with("timens:"));
            assert!(start_val.contains(";ticks="));
            for fact_key in [
                "security_principal",
                "namespace_context_digest",
                "executable_digest",
                "arguments_digest",
                "security_context_digest",
            ] {
                assert!(
                    projected["execution_context"].get(fact_key).is_some(),
                    "projected execution_context missing {fact_key}"
                );
            }
        } else {
            // On macOS, namespace_context_digest and security_context_digest are
            // unsupported in v0, so projection into a 5-context-fact session record
            // must fail closed.
            assert!(
                proj_res.is_err(),
                "macOS observation without Linux namespace/secctx digests must fail session projection closed"
            );
            let _ = ZERO_DIGEST;
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_non_leader_thread_tid_reports_not_present_tid_not_thread_group_leader() {
        use std::sync::mpsc;

        let (tid_tx, tid_rx) = mpsc::channel::<u32>();
        let (done_tx, done_rx) = mpsc::channel::<()>();

        let worker = std::thread::spawn(move || {
            let tid = unsafe { libc::syscall(libc::SYS_gettid) } as u32;
            tid_tx.send(tid).expect("send worker tid");
            let _ = done_rx.recv();
        });

        let worker_tid = tid_rx.recv().expect("receive worker tid");
        assert_ne!(
            worker_tid,
            std::process::id(),
            "worker thread TID must differ from thread-group leader PID"
        );

        let hs = sample_host_scope();
        let opts = ObservationOptions {
            observed_at: Some("2026-11-06T12:00:00Z".to_string()),
            ..ObservationOptions::default()
        };
        if verify_proc_sys_overmount_refuses_closed(worker_tid, &hs, &opts) {
            let _ = done_tx.send(());
            worker.join().expect("join worker thread");
            return;
        }
        let obs = observe_process(worker_tid, &hs, &opts)
            .expect("observing non-leader TID must return structured not-present observation");
        assert_eq!(
            obs["process"]["process_state"],
            json!({
                "status": "not-present",
                "reason": "tid-not-thread-group-leader",
            })
        );
        assert_eq!(obs["process"]["start_identity"]["status"], "not-present");

        let herr = open_continuity_handle(worker_tid, &hs, &opts)
            .expect_err("continuity handle must refuse non-leader thread TID");
        assert_eq!(herr.status, "not-present");

        let _ = done_tx.send(());
        worker.join().expect("join worker thread");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_symlinked_executable_and_live_empty_argv0_all_eight_residues() {
        use std::ffi::{CStr, CString};
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::symlink;
        use std::path::PathBuf;

        struct ScopedTempDir(PathBuf);
        impl ScopedTempDir {
            fn new() -> Self {
                let mut tmpl = std::env::temp_dir();
                tmpl.push("sysprims-obs-XXXXXX");
                let mut raw = tmpl.as_os_str().as_bytes().to_vec();
                raw.push(0);
                let ptr = unsafe { libc::mkdtemp(raw.as_mut_ptr() as *mut libc::c_char) };
                assert!(!ptr.is_null(), "mkdtemp failed");
                let cstr = unsafe { CStr::from_ptr(ptr) };
                Self(PathBuf::from(std::ffi::OsStr::from_bytes(cstr.to_bytes())))
            }
            fn path(&self) -> &Path {
                &self.0
            }
        }
        impl Drop for ScopedTempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let sleep_path = ["/bin/sleep", "/usr/bin/sleep"]
            .into_iter()
            .find(|p| Path::new(p).exists())
            .expect("sleep binary present at /bin/sleep or /usr/bin/sleep");
        let expected_exe_digest =
            derive_executable_digest(&std::fs::read(sleep_path).expect("read sleep binary"));

        let tmp = ScopedTempDir::new();
        let hs = sample_host_scope();
        let opts = ObservationOptions {
            observed_at: Some("2026-11-06T12:00:00Z".to_string()),
            ..ObservationOptions::default()
        }
        .with_all_context_facts();

        // 1. Symlinked executable resolves cleanly and hashes target image under O_NOFOLLOW | O_NONBLOCK.
        let sym_path = tmp.path().join("sleep-symlink");
        symlink(sleep_path, &sym_path).expect("create sleep symlink");
        let mut sym_child = Command::new(&sym_path)
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn symlink child");
        let sym_obs = observe_process(sym_child.id(), &hs, &opts).expect("observe symlink child");
        let _ = sym_child.kill();
        let _ = sym_child.wait();

        assert_eq!(
            sym_obs["process"]["execution_context"]["executable_digest"]["status"],
            "observed"
        );
        assert_eq!(
            sym_obs["process"]["execution_context"]["executable_digest"]["digest"],
            expected_exe_digest
        );
        let expected_sym_args = derive_arguments_digest(&[sym_path.as_os_str().as_bytes(), b"30"]);
        assert_eq!(
            sym_obs["process"]["execution_context"]["arguments_digest"]["status"],
            "observed"
        );
        assert_eq!(
            sym_obs["process"]["execution_context"]["arguments_digest"]["digest"],
            expected_sym_args
        );

        // 2. Live kernel execve with empty argv[0] = "" across all 8 exec_path length residues
        // refuses arguments_digest fail-closed with ambiguous-kern-procargs2-padding without
        // consuming subsequent argv or SECRET_ENV bytes.
        for residue in 0..8usize {
            let base_name = format!("s{}", "x".repeat(residue));
            let mut link_path = tmp.path().join(base_name);
            let cur_len = link_path.as_os_str().as_bytes().len();
            let extra = (8 + residue - (cur_len % 8)) % 8;
            if extra > 0 {
                let mut os = link_path.into_os_string();
                os.push("y".repeat(extra));
                link_path = std::path::PathBuf::from(os);
            }
            assert_eq!(link_path.as_os_str().as_bytes().len() % 8, residue);
            symlink(sleep_path, &link_path).expect("create residue symlink");

            let c_exec = CString::new(link_path.as_os_str().as_bytes()).expect("c_exec");
            let c_arg0 = CString::new("").expect("empty arg0");
            let c_arg1 = CString::new("30").expect("arg1");
            let c_env0 = CString::new("SECRET_ENV=NEVER_IN_ARGV").expect("env0");
            let argv_ptrs: [*const libc::c_char; 3] =
                [c_arg0.as_ptr(), c_arg1.as_ptr(), std::ptr::null()];
            let envp_ptrs: [*const libc::c_char; 2] = [c_env0.as_ptr(), std::ptr::null()];

            let raw_pid = unsafe { libc::fork() };
            assert!(raw_pid >= 0, "fork failed for residue {residue}");
            if raw_pid == 0 {
                unsafe {
                    libc::execve(c_exec.as_ptr(), argv_ptrs.as_ptr(), envp_ptrs.as_ptr());
                    libc::_exit(127);
                }
            }

            // Wait briefly until child has completed execve into sleep (or observe directly
            // once exec image matches sleep_path).
            let child_pid = raw_pid as u32;
            let mut final_obs = None;
            for _ in 0..50 {
                let obs =
                    observe_process(child_pid, &hs, &opts).expect("observe empty-argv0 child");
                if obs["process"]["execution_context"]["executable_digest"]["digest"]
                    == expected_exe_digest
                {
                    final_obs = Some(obs);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }

            unsafe {
                let _ = libc::kill(raw_pid, libc::SIGKILL);
                let mut status: libc::c_int = 0;
                let _ = libc::waitpid(raw_pid, &mut status, 0);
            }

            let obs = final_obs.expect("child completed execve into sleep");
            assert_eq!(
                obs["process"]["execution_context"]["arguments_digest"],
                json!({
                    "status": "error",
                    "reason": "ambiguous-kern-procargs2-padding",
                }),
                "residue {residue} must refuse empty argv[0] fail-closed"
            );
            assert_eq!(
                obs["process"]["execution_context"]["executable_digest"]["digest"],
                expected_exe_digest
            );
        }
    }
}
