use std::fs::File;
use std::net::TcpListener;
use std::time::{SystemTime, UNIX_EPOCH};

use sysprims_proc::{list_fds, FdFilter, FdKind};

#[test]
fn list_fds_includes_open_file_and_socket_for_self() {
    let pid = std::process::id();

    // Create a temp file and keep it open.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let file_path = std::env::temp_dir().join(format!("sysprims-fds-test-{pid}-{now}.txt"));
    let _file = File::create(&file_path).expect("create temp file");

    // Create a listener and keep it open.
    let _listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::PermissionDenied {
                eprintln!("skipping: TcpListener bind denied: {err}");
                return;
            }
            panic!("bind: {err}");
        }
    };

    let snapshot = match list_fds(pid, None) {
        Ok(s) => s,
        Err(sysprims_core::SysprimsError::NotSupported { .. }) => {
            eprintln!("SKIP: list_fds returned NotSupported on this platform");
            return;
        }
        Err(e) => panic!("list_fds: {e}"),
    };

    let file_path_s = file_path.to_string_lossy();
    let has_file = snapshot.fds.iter().any(|fd| {
        fd.kind == FdKind::File
            && fd
                .path
                .as_deref()
                .is_some_and(|p| p.contains(file_path_s.as_ref()))
    });
    assert!(
        has_file,
        "did not find open file path in fds; warnings={:?}",
        snapshot.warnings
    );

    let has_socket = snapshot.fds.iter().any(|fd| fd.kind == FdKind::Socket);
    assert!(
        has_socket,
        "did not find any socket fds; warnings={:?}",
        snapshot.warnings
    );
}

#[test]
fn list_fds_filter_by_kind_socket_only() {
    let pid = std::process::id();
    let _listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::PermissionDenied {
                eprintln!("skipping: TcpListener bind denied: {err}");
                return;
            }
            panic!("bind: {err}");
        }
    };

    let filter = FdFilter {
        kind: Some(FdKind::Socket),
    };

    let snapshot = match list_fds(pid, Some(&filter)) {
        Ok(s) => s,
        Err(sysprims_core::SysprimsError::NotSupported { .. }) => {
            eprintln!("SKIP: list_fds returned NotSupported on this platform");
            return;
        }
        Err(e) => panic!("list_fds: {e}"),
    };

    assert!(
        snapshot.fds.iter().all(|fd| fd.kind == FdKind::Socket),
        "expected only socket fds"
    );
}
