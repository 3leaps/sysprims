//! Signal handling for guard loops.
//!
//! Wraps `rsfulmen::signals::SignalManager` with a stop-flag pattern
//! suitable for tick-based guard loops: register SIGINT/SIGTERM handlers
//! that set a shared flag, spawn the listener thread, and poll `should_stop()`
//! between ticks.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use rsfulmen::signals::testing::SignalInjector;
pub use rsfulmen::signals::{DoubleTapConfig, SignalManager, SignalManagerError};

use crate::SysprimsResult;

/// Write-only handle to a guard stop flag.
///
/// Can only set the flag to `true` (request stop). Cannot read or reset it.
/// This prevents external callers from observing or clearing internal state.
#[derive(Clone)]
pub struct StopFlagHandle(Arc<AtomicBool>);

impl StopFlagHandle {
    /// Set the stop flag to `true`.
    pub fn set(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Signal controller for guard-style tick loops.
///
/// Sets up SIGINT/SIGTERM handling with double-tap support and exposes
/// a `should_stop()` flag that the loop checks between ticks.
///
/// Dropping `GuardSignals` stops the listener thread and joins it.
///
/// # Example
///
/// ```no_run
/// use std::time::Duration;
/// use sysprims_core::guard_signals::GuardSignals;
/// use sysprims_core::time::Tick;
///
/// let signals = GuardSignals::start().unwrap();
/// let mut tick = Tick::new(Duration::from_secs(5)).unwrap();
/// while !signals.should_stop() {
///     // do guard work …
///     tick.sleep_until_next();
/// }
/// ```
pub struct GuardSignals {
    stop_flag: Arc<AtomicBool>,
    manager: SignalManager,
    /// Used internally by Drop to inject a synthetic SIGTERM for clean shutdown.
    injector: SignalInjector,
    listener_thread: Option<JoinHandle<()>>,
}

impl GuardSignals {
    /// Set up signal handlers and start the listener thread.
    ///
    /// Registers SIGINT and SIGTERM handlers that set the stop flag,
    /// enables double-tap (catalog defaults), and spawns a background
    /// thread running `SignalManager::listen()`.
    ///
    /// If the listener exits early (error or unexpected return), the
    /// stop flag is set so the guard loop does not run without signal
    /// responsiveness.
    pub fn start() -> SysprimsResult<Self> {
        let manager = SignalManager::new();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let injector = SignalInjector::new(&manager);

        // Register SIGTERM handler
        let flag = Arc::clone(&stop_flag);
        let mgr = manager.clone();
        let _term_reg = manager
            .handle(crate::signals::SIGTERM, move || {
                flag.store(true, Ordering::SeqCst);
                mgr.stop();
                Ok(())
            })
            .map_err(|e| crate::SysprimsError::internal(format!("SIGTERM handler: {e}")))?;

        // Register SIGINT handler
        let flag = Arc::clone(&stop_flag);
        let mgr = manager.clone();
        let _int_reg = manager
            .handle(crate::signals::SIGINT, move || {
                flag.store(true, Ordering::SeqCst);
                mgr.stop();
                Ok(())
            })
            .map_err(|e| crate::SysprimsError::internal(format!("SIGINT handler: {e}")))?;

        // Enable double-tap Ctrl+C (catalog defaults: 2s window, exit 130)
        manager.enable_double_tap(DoubleTapConfig::from_catalog());

        // Spawn listener thread — routes OS signals to registered handlers.
        // The registrations (_term_reg, _int_reg) are moved into the thread
        // so they stay alive as long as the listener runs.
        // If listen() exits early (error or unexpected return), set the stop
        // flag so the guard loop doesn't continue deaf to signals.
        let listener_mgr = manager.clone();
        let listener_flag = Arc::clone(&stop_flag);
        let handle = thread::Builder::new()
            .name("guard-signals".into())
            .spawn(move || {
                // Keep registrations alive for the lifetime of the listener
                let _term = _term_reg;
                let _int = _int_reg;
                if let Err(_e) = listener_mgr.listen() {
                    // Listener failed — mark stop so guard loop won't run deaf
                    listener_flag.store(true, Ordering::SeqCst);
                }
            })
            .map_err(|e| crate::SysprimsError::internal(format!("signal thread: {e}")))?;

        Ok(Self {
            stop_flag,
            manager,
            injector,
            listener_thread: Some(handle),
        })
    }

    /// Check whether a shutdown signal has been received.
    #[inline]
    pub fn should_stop(&self) -> bool {
        self.stop_flag.load(Ordering::SeqCst)
    }

    /// Request stop (useful for programmatic shutdown, e.g. max-iterations).
    pub fn request_stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        self.manager.stop();
    }

    /// Access the underlying `SignalManager` (for shutdown hooks or test injection).
    pub fn manager(&self) -> &SignalManager {
        &self.manager
    }

    /// Get a write handle to the shared stop flag.
    ///
    /// Used by [`GuardStopHandle`](crate) to request stop from another thread
    /// without holding a reference to `GuardSignals`.
    pub fn stop_flag_handle(&self) -> StopFlagHandle {
        StopFlagHandle(Arc::clone(&self.stop_flag))
    }
}

impl Drop for GuardSignals {
    fn drop(&mut self) {
        // Ensure the listener thread shuts down cleanly.
        //
        // Key constraint: rsfulmen's listen() resets its internal stop_flag
        // to false on entry. Calling manager.stop() before listen() enters
        // its loop is a no-op — the flag gets cleared and the thread hangs.
        //
        // Solution: inject a synthetic SIGTERM into the manager's mpsc
        // channel. This is timing-independent:
        //   - If listen() hasn't started yet: the signal queues in the
        //     channel and gets drained once listen() enters its loop.
        //   - If listen() is already running: it drains the signal on
        //     the next 25ms poll cycle.
        //   - If listen() already exited: the send may fail (channel
        //     closed), which is fine — the thread is already done.
        //
        // In all cases, our registered SIGTERM handler fires, which sets
        // our stop_flag and calls manager.stop(), cleanly exiting listen().
        if !self.stop_flag.load(Ordering::SeqCst) {
            let _ = self.injector.inject(crate::signals::SIGTERM);
        }
        if let Some(handle) = self.listener_thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_injector(gs: &GuardSignals) -> SignalInjector {
        SignalInjector::new(gs.manager())
    }

    #[test]
    fn test_guard_signals_starts_and_drops_cleanly() {
        let gs = GuardSignals::start();
        assert!(gs.is_ok(), "GuardSignals::start() should succeed");
        let gs = gs.unwrap();
        assert!(!gs.should_stop(), "should not be stopped initially");
        // Drop injects synthetic SIGTERM, joins listener — must not hang
        drop(gs);
    }

    #[test]
    fn test_guard_signals_request_stop_and_drop() {
        let gs = GuardSignals::start().unwrap();
        let inj = test_injector(&gs);
        // Wait for listener to be ready so request_stop is reliable
        inj.wait_for_listen(Duration::from_secs(2))
            .expect("listener should start");
        assert!(!gs.should_stop());
        gs.request_stop();
        assert!(gs.should_stop(), "should be stopped after request_stop()");
        // Drop skips inject since flag is set, joins cleanly
        drop(gs);
    }

    #[test]
    fn test_guard_signals_injected_sigterm() {
        let gs = GuardSignals::start().unwrap();
        let inj = test_injector(&gs);

        // Wait for listener thread to be ready
        inj.wait_for_listen(Duration::from_secs(2))
            .expect("listener should start");

        // Inject SIGTERM
        inj.inject(crate::signals::SIGTERM)
            .expect("inject should succeed");

        // Give handler a moment to fire
        thread::sleep(Duration::from_millis(100));
        assert!(gs.should_stop(), "should be stopped after injected SIGTERM");
        // Drop joins the (now-stopped) listener thread
        drop(gs);
    }
}
