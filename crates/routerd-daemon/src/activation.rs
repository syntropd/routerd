use anyhow::Result;
use rustix::fd::BorrowedFd;
use rustix::fs::{fcntl_getfd, fcntl_setfd, FdFlags};
use rustix::net::{getsockname, SocketAddrAny};
use std::env;
use std::net::TcpListener as StdTcpListener;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::FromRawFd;
use std::os::unix::net::UnixListener as StdUnixListener;
use std::path::Path;
use tokio::net::{TcpListener, UnixListener};
use tracing::{info, warn};

pub const SD_LISTEN_FDS_START: i32 = 3;
const SOCKET_VARLINK: &str = "io.syntrop.Router1";
const SOCKET_GATEWAY_UNIX: &str = "router.sock";

#[derive(Default)]
pub struct ActivatedSockets {
    pub tcp_gateways: Vec<TcpListener>,
    pub unix_gateway: Option<UnixListener>,
    pub varlink: Option<UnixListener>,
}

pub fn is_socket_activated() -> bool {
    let pid_matches = env::var("LISTEN_PID")
        .ok()
        .and_then(|p| p.parse::<u32>().ok())
        .map(|pid| pid == std::process::id())
        .unwrap_or(false);

    let has_fds = env::var("LISTEN_FDS")
        .ok()
        .and_then(|f| f.parse::<i32>().ok())
        .map(|count| count > 0)
        .unwrap_or(false);

    pid_matches && has_fds
}

fn classify_unix_socket(path: &Path) -> Option<&'static str> {
    let name = path.file_name().and_then(|n| n.to_str())?;
    match name {
        SOCKET_VARLINK => Some("varlink"),
        SOCKET_GATEWAY_UNIX => Some("gateway-unix"),
        _ => None,
    }
}

pub fn check_and_adopt_sockets() -> Result<ActivatedSockets> {
    let mut sockets = ActivatedSockets::default();

    if !is_socket_activated() {
        return Ok(sockets);
    }

    let fds_count: i32 = env::var("LISTEN_FDS")
        .unwrap_or_default()
        .parse()
        .unwrap_or(0);

    info!(
        "Adopting {} socket activation descriptor(s) from systemd",
        fds_count
    );

    for i in 0..fds_count {
        let fd_raw = SD_LISTEN_FDS_START + i;
        let borrowed_fd = unsafe { BorrowedFd::borrow_raw(fd_raw) };

        if let Ok(flags) = fcntl_getfd(borrowed_fd) {
            let _ = fcntl_setfd(borrowed_fd, flags | FdFlags::CLOEXEC);
        }

        let sock_addr = match getsockname(borrowed_fd) {
            Ok(addr) => addr,
            Err(e) => {
                warn!("Failed getsockname on fd {}: {}", fd_raw, e);
                continue;
            }
        };

        match sock_addr {
            SocketAddrAny::Unix(addr) => {
                let std_unix = unsafe { StdUnixListener::from_raw_fd(fd_raw) };
                std_unix.set_nonblocking(true)?;
                let tokio_unix = UnixListener::from_std(std_unix)?;
                let path = addr
                    .path()
                    .map(|p| Path::new(std::ffi::OsStr::from_bytes(p.to_bytes())).to_path_buf());
                let kind = path.as_deref().and_then(classify_unix_socket);

                match kind {
                    Some("varlink") => {
                        info!("Adopted Varlink socket on fd {}", fd_raw);
                        sockets.varlink = Some(tokio_unix);
                    }
                    Some("gateway-unix") => {
                        info!("Adopted Gateway Unix socket on fd {}", fd_raw);
                        sockets.unix_gateway = Some(tokio_unix);
                    }
                    _ => {
                        // Positional fallback
                        if sockets.unix_gateway.is_none() {
                            info!("Adopted Gateway Unix socket by fallback on fd {}", fd_raw);
                            sockets.unix_gateway = Some(tokio_unix);
                        } else if sockets.varlink.is_none() {
                            info!("Adopted Varlink socket by fallback on fd {}", fd_raw);
                            sockets.varlink = Some(tokio_unix);
                        } else {
                            warn!("Unassigned Unix socket descriptor on fd {}", fd_raw);
                        }
                    }
                }
            }
            SocketAddrAny::V4(addr) => {
                let std_tcp = unsafe { StdTcpListener::from_raw_fd(fd_raw) };
                std_tcp.set_nonblocking(true)?;
                let tokio_tcp = TcpListener::from_std(std_tcp)?;
                info!("Adopted IPv4 Gateway socket on fd {} ({})", fd_raw, addr);
                sockets.tcp_gateways.push(tokio_tcp);
            }
            SocketAddrAny::V6(addr) => {
                let std_tcp = unsafe { StdTcpListener::from_raw_fd(fd_raw) };
                std_tcp.set_nonblocking(true)?;
                let tokio_tcp = TcpListener::from_std(std_tcp)?;
                info!("Adopted IPv6 Gateway socket on fd {} ({})", fd_raw, addr);
                sockets.tcp_gateways.push(tokio_tcp);
            }
            _ => warn!("Unrecognized socket family on fd {}", fd_raw),
        }
    }

    Ok(sockets)
}
