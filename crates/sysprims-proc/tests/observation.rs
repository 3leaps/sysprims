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

    use super::{sample_host_scope, ZERO_DIGEST};

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
        assert_eq!(
            outcome_zombie_live_req,
            ComparisonOutcome::Mismatch("process-not-live".to_string())
        );

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
        assert_eq!(outcome_zombie_relaxed, ComparisonOutcome::Match);

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
}
