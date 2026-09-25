use routerd_daemon::activation::{check_and_adopt_sockets, is_socket_activated, SD_LISTEN_FDS_START};
use std::env;
use std::net::TcpListener as StdTcpListener;
use std::os::unix::io::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_socket_activation_detection() {
    let current_pid = std::process::id();
    env::set_var("LISTEN_PID", current_pid.to_string());
    env::set_var("LISTEN_FDS", "2");

    assert!(is_socket_activated());

    // Mismatched PID should fail
    env::set_var("LISTEN_PID", (current_pid + 9999).to_string());
    assert!(!is_socket_activated());

    env::remove_var("LISTEN_PID");
    env::remove_var("LISTEN_FDS");
    assert!(!is_socket_activated());
}

#[test]
fn test_socket_activation_adoption_child_process() {
    // If invoked as the child process, run the adoption test directly
    if env::var("ROUTERD_ACTIVATION_TEST_CHILD").as_deref() == Ok("1") {
        env::set_var("LISTEN_PID", std::process::id().to_string());
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let adopted = check_and_adopt_sockets().expect("Socket adoption should succeed");
            assert_eq!(adopted.tcp_gateways.len(), 1);
            assert!(adopted.unix_gateway.is_some() || adopted.varlink.is_some());
        });
        std::process::exit(0);
    }

    let dir = tempdir().unwrap();
    let unix_path = dir.path().join("router.sock");

    let tcp_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let unix_listener = std::os::unix::net::UnixListener::bind(&unix_path).unwrap();

    let tcp_fd = tcp_listener.as_raw_fd();
    let unix_fd = unix_listener.as_raw_fd();

    let current_exe = env::current_exe().unwrap();
    let mut cmd = Command::new(current_exe);
    cmd.arg("test_socket_activation_adoption_child_process");
    cmd.arg("--nocapture");
    cmd.env("ROUTERD_ACTIVATION_TEST_CHILD", "1");
    cmd.env("LISTEN_FDS", "2");

    unsafe {
        cmd.pre_exec(move || {
            let pid = libc::getpid().to_string();
            let pid_cstr = std::ffi::CString::new(pid).unwrap();
            let key_cstr = std::ffi::CString::new("LISTEN_PID").unwrap();
            libc::setenv(key_cstr.as_ptr(), pid_cstr.as_ptr(), 1);

            if tcp_fd != SD_LISTEN_FDS_START {
                if libc::dup2(tcp_fd, SD_LISTEN_FDS_START) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            if unix_fd != SD_LISTEN_FDS_START + 1 {
                if libc::dup2(unix_fd, SD_LISTEN_FDS_START + 1) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }

            libc::fcntl(SD_LISTEN_FDS_START, libc::F_SETFD, 0);
            libc::fcntl(SD_LISTEN_FDS_START + 1, libc::F_SETFD, 0);

            Ok(())
        });
    }

    let output = cmd.output().expect("Failed to execute child process");
    if !output.status.success() {
        panic!(
            "Child process failed with exit code {:?}!\nstdout: {}\nstderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
