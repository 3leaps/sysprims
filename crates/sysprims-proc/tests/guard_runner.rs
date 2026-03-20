use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sysprims_proc::{
    CpuMode, GuardAction, GuardConfig, GuardPreset, GuardRule, GuardRunner, GuardRunnerConfig,
    StopReason,
};

fn observation_config(
    root_pid: u32,
    interval: Duration,
    max_iterations: Option<u64>,
) -> GuardRunnerConfig {
    GuardRunnerConfig {
        guard: GuardConfig {
            rule: GuardRule {
                root_pid,
                max_levels: 1,
                filter: None,
                cpu_mode: CpuMode::Lifetime,
                sample_duration: None,
            },
            action: GuardAction::KillDescendants {
                signal: 15,
                cascade: false,
            },
            action_enabled: false,
            max_targets: 8,
        },
        interval,
        max_iterations,
    }
}

#[test]
fn test_guard_runner_max_iterations() {
    let self_pid = std::process::id();
    let config = observation_config(self_pid, Duration::from_millis(50), Some(3));

    let mut runner = GuardRunner::new(config).unwrap();
    let event_count = Arc::new(AtomicU64::new(0));
    let count = Arc::clone(&event_count);

    let summary = runner
        .run(
            |_event| {
                count.fetch_add(1, Ordering::SeqCst);
            },
            |_tick, _err| {},
        )
        .unwrap();

    assert_eq!(summary.ticks, 3);
    assert_eq!(summary.stop_reason, StopReason::MaxIterations);
    assert_eq!(event_count.load(Ordering::SeqCst), 3);
}

#[test]
fn test_guard_runner_stop_handle_from_thread() {
    let self_pid = std::process::id();
    // No max_iterations — will run forever unless stopped
    let config = observation_config(self_pid, Duration::from_millis(50), None);

    let mut runner = GuardRunner::new(config).unwrap();
    let handle = runner.stop_handle();

    // Stop from another thread after ~150ms (should allow 2-3 ticks)
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        handle.stop();
    });

    let start = Instant::now();
    let summary = runner.run(|_event| {}, |_tick, _err| {}).unwrap();

    assert_eq!(summary.stop_reason, StopReason::Requested);
    assert!(summary.ticks >= 1, "should have run at least 1 tick");
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "should stop promptly after handle.stop()"
    );
}

#[test]
fn test_guard_runner_stop_handle_is_cloneable() {
    let self_pid = std::process::id();
    let config = observation_config(self_pid, Duration::from_millis(50), None);

    let mut runner = GuardRunner::new(config).unwrap();
    let handle1 = runner.stop_handle();
    let handle2 = handle1.clone();

    // Either clone can stop the runner
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        handle2.stop();
    });

    let summary = runner.run(|_event| {}, |_tick, _err| {}).unwrap();
    assert_eq!(summary.stop_reason, StopReason::Requested);

    // handle1 is still valid (tests Clone doesn't break the original)
    drop(handle1);
}

#[test]
fn test_guard_runner_tick_scheduling() {
    let self_pid = std::process::id();
    let config = observation_config(self_pid, Duration::from_millis(100), Some(3));

    let mut runner = GuardRunner::new(config).unwrap();
    let start = Instant::now();

    let summary = runner.run(|_event| {}, |_tick, _err| {}).unwrap();

    let elapsed = start.elapsed();
    assert_eq!(summary.ticks, 3);
    // 3 ticks at 100ms intervals: first tick immediate, then 2 sleeps
    assert!(
        elapsed >= Duration::from_millis(150),
        "should take at least ~200ms for 3 ticks at 100ms: {:?}",
        elapsed
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "should not take more than 2s: {:?}",
        elapsed
    );
}

#[test]
fn test_guard_runner_error_callback() {
    // PID 99999 almost certainly doesn't exist — guard_step will error
    let config = observation_config(99999, Duration::from_millis(50), Some(2));

    let mut runner = GuardRunner::new(config).unwrap();
    let error_count = Arc::new(AtomicU64::new(0));
    let count = Arc::clone(&error_count);
    let event_count = Arc::new(AtomicU64::new(0));
    let ecount = Arc::clone(&event_count);

    let summary = runner
        .run(
            |_event| {
                ecount.fetch_add(1, Ordering::SeqCst);
            },
            |_tick, _err| {
                count.fetch_add(1, Ordering::SeqCst);
            },
        )
        .unwrap();

    assert_eq!(summary.ticks, 2);
    assert_eq!(summary.stop_reason, StopReason::MaxIterations);
    assert_eq!(error_count.load(Ordering::SeqCst), 2);
    assert_eq!(event_count.load(Ordering::SeqCst), 0);
}

#[test]
fn test_guard_runner_rejects_zero_interval() {
    let config = observation_config(std::process::id(), Duration::ZERO, Some(1));
    let result = GuardRunner::new(config);
    assert!(result.is_err(), "zero interval should be rejected");
}

#[test]
fn test_guard_runner_single_tick() {
    let self_pid = std::process::id();
    let config = observation_config(self_pid, Duration::from_millis(50), Some(1));

    let mut runner = GuardRunner::new(config).unwrap();
    let mut saw_event = false;

    let summary = runner
        .run(
            |event| {
                saw_event = true;
                assert_eq!(event.killed, 0);
                assert_eq!(event.targeted, 0);
            },
            |_tick, _err| {
                panic!("should not error on self PID");
            },
        )
        .unwrap();

    assert!(saw_event, "should have received at least one event");
    assert_eq!(summary.ticks, 1);
    assert_eq!(summary.stop_reason, StopReason::MaxIterations);
}

#[test]
fn test_guard_preset_intervals() {
    assert_eq!(GuardPreset::Interactive.interval(), Duration::from_secs(3));
    assert_eq!(GuardPreset::Background.interval(), Duration::from_secs(180));
    assert_eq!(GuardPreset::Watchdog.interval(), Duration::from_secs(300));
}

#[test]
fn test_guard_preset_sample_durations() {
    assert_eq!(
        GuardPreset::Interactive.sample_duration(),
        Duration::from_secs(2)
    );
    assert_eq!(
        GuardPreset::Background.sample_duration(),
        Duration::from_secs(3)
    );
    assert_eq!(
        GuardPreset::Watchdog.sample_duration(),
        Duration::from_secs(5)
    );
}

#[test]
fn test_guard_runner_with_preset() {
    let self_pid = std::process::id();
    let preset = GuardPreset::Interactive;

    let config = GuardRunnerConfig {
        guard: GuardConfig {
            rule: GuardRule {
                root_pid: self_pid,
                max_levels: 1,
                filter: None,
                cpu_mode: CpuMode::Lifetime,
                sample_duration: Some(preset.sample_duration()),
            },
            action: GuardAction::KillDescendants {
                signal: 15,
                cascade: false,
            },
            action_enabled: false,
            max_targets: 8,
        },
        interval: preset.interval(),
        max_iterations: Some(1),
    };

    let mut runner = GuardRunner::new(config).unwrap();
    let summary = runner.run(|_| {}, |_, _| {}).unwrap();
    assert_eq!(summary.ticks, 1);
}
