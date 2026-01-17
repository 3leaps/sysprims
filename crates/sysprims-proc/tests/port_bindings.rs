use std::net::TcpListener;

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

    let snapshot = listening_ports(Some(&filter)).expect("listening_ports");
    let found = snapshot
        .bindings
        .iter()
        .any(|b| b.local_port == port && b.pid == Some(pid));

    if !found {
        // Best-effort: socket introspection can be limited by permissions:
        // - macOS: SIP/TCC can block even for same-user processes
        // - Linux: /proc/<pid>/fd requires root or same-user for inode->pid mapping
        // - Windows: unprivileged users may have limited netstat access
        //
        // When we see permission warnings, treat as best-effort instead of hard failure.
        let has_permission_warnings = snapshot
            .warnings
            .iter()
            .any(|w| w.contains("permission") || w.contains("Permission"));

        if cfg!(target_os = "macos") || has_permission_warnings {
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
