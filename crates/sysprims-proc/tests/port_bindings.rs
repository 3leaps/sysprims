use std::net::TcpListener;

use sysprims_core::SysprimsError;
use sysprims_proc::{listening_ports, PortFilter, Protocol};

#[test]
fn test_listening_ports_self_listener_tcp() {
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::PermissionDenied {
                // Some sandboxed environments disallow even loopback binds.
                eprintln!("skipping: TcpListener bind denied: {err}");
                return;
            }
            panic!("bind: {err}");
        }
    };
    let port = listener.local_addr().expect("local_addr").port();
    let pid = std::process::id();

    let filter = PortFilter {
        protocol: Some(Protocol::Tcp),
        local_port: Some(port),
    };

    let snapshot = match listening_ports(Some(&filter)) {
        Ok(s) => s,
        Err(SysprimsError::NotSupported { .. }) => {
            eprintln!("SKIP: listening_ports returned NotSupported (container/musl environment)");
            return;
        }
        Err(e) => panic!("listening_ports: {e}"),
    };
    let found = snapshot
        .bindings
        .iter()
        .any(|b| b.local_port == port && b.pid == Some(pid));

    if !found {
        // On macOS, the current process should be introspectable without special privileges.
        // If we can't see our own listener, treat as a bug.
        if cfg!(target_os = "macos") {
            panic!(
                "Did not find self listener pid={} port={}; warnings={:?} bindings={}",
                pid,
                port,
                snapshot.warnings,
                snapshot.bindings.len()
            );
        }

        // Best-effort: socket introspection can be limited by permissions:
        // - Linux: /proc/<pid>/fd requires root or same-user for inode->pid mapping
        // - Windows: unprivileged users may have limited netstat access
        //
        // When we see permission warnings, treat as best-effort instead of hard failure.
        let has_permission_warnings = snapshot
            .warnings
            .iter()
            .any(|w| w.contains("permission") || w.contains("Permission"));

        if has_permission_warnings {
            eprintln!(
                "Did not find self listener pid={} port={}; warnings={:?} bindings={} (best-effort: permission-limited)",
                pid,
                port,
                snapshot.warnings,
                snapshot.bindings.len()
            );
            return;
        }
    }

    assert!(
        found,
        "Did not find self listener pid={} port={}; warnings={:?} bindings={}",
        pid,
        port,
        snapshot.warnings,
        snapshot.bindings.len()
    );
}
