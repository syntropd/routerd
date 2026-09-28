use routerd_daemon::notify::{notify_systemd_ready, notify_systemd_stopping, notify_systemd_watchdog};
use std::env;
use std::os::unix::net::UnixDatagram;
use tempfile::tempdir;

#[test]
fn test_systemd_notify_lifecycle() {
    let dir = tempdir().unwrap();
    let sock_path = dir.path().join("notify.sock");

    let server_sock = UnixDatagram::bind(&sock_path).unwrap();
    server_sock.set_nonblocking(true).unwrap();

    env::set_var("NOTIFY_SOCKET", sock_path.to_str().unwrap());

    // 1. Test READY
    notify_systemd_ready();
    let mut buf = [0u8; 512];
    let (n, _) = server_sock.recv_from(&mut buf).expect("Should receive READY notification");
    let msg = String::from_utf8_lossy(&buf[..n]);
    assert!(msg.contains("READY=1"));

    // 2. Test WATCHDOG
    notify_systemd_watchdog();
    let (n, _) = server_sock.recv_from(&mut buf).expect("Should receive WATCHDOG notification");
    let msg = String::from_utf8_lossy(&buf[..n]);
    assert!(msg.contains("WATCHDOG=1"));

    // 3. Test STOPPING
    notify_systemd_stopping();
    let (n, _) = server_sock.recv_from(&mut buf).expect("Should receive STOPPING notification");
    let msg = String::from_utf8_lossy(&buf[..n]);
    assert!(msg.contains("STOPPING=1"));

    env::remove_var("NOTIFY_SOCKET");
}
