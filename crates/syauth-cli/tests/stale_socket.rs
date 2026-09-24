//! Regression: a stale `pair-confirm.sock` inode left behind by a killed GUI
//! session must not block the next GUI session.
//!
//! The confirmation client unlinks the well-known DeskUnlock socket path
//! before binding it, so a stale inode with no listener is always replaced.
//! This test pins that behaviour end to end.

#![cfg(unix)]

use std::{
    os::unix::fs::FileTypeExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn a_stale_confirmation_socket_does_not_block_the_next_gui_session() {
    let runtime = tempfile::tempdir().expect("tempdir");
    let dir = runtime.path().join("syauth");
    std::fs::create_dir_all(&dir).expect("create runtime dir");
    let socket = dir.join("pair-confirm.sock");
    // Exactly what a killed GUI leaves behind: the path exists, no listener.
    std::fs::write(&socket, b"stale inode").expect("write stale file");

    let mut child = Command::new(env!("CARGO_BIN_EXE_syauth"))
        .args(["pair", "--gui"])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn `syauth pair --gui`");

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut bound = false;
    while Instant::now() < deadline {
        let is_socket = std::fs::metadata(&socket).is_ok_and(|meta| meta.file_type().is_socket());
        // A live listener accepts a client connection.
        if is_socket && std::os::unix::net::UnixStream::connect(&socket).is_ok() {
            bound = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let _ = child.kill();
    let _ = child.wait();

    assert!(bound, "the GUI session must bind successfully over a stale pair-confirm.sock inode");
}
