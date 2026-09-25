use rustix::net::{
    sendto_unix, socket, AddressFamily, SendFlags, SocketAddrUnix, SocketType,
};
use std::env;
use std::io;
use std::path::Path;
use tracing::{info, warn};

const NOTIFY_MAX: usize = 8 * 1024 * 1024;

fn notify_address(socket_path: &str) -> Option<SocketAddrUnix> {
    if let Some(name) = socket_path.strip_prefix('@') {
        if name.is_empty() || name.as_bytes().contains(&0) {
            return None;
        }
        SocketAddrUnix::new_abstract_name(name.as_bytes()).ok()
    } else if socket_path.is_empty() {
        None
    } else {
        SocketAddrUnix::new(Path::new(socket_path)).ok()
    }
}

#[allow(dead_code)]
fn sanitize_value(s: &str) -> String {
    s.chars().filter(|&c| c != '\n' && c != '\r').collect()
}

pub fn notify_systemd_ready() {
    if let Err(e) = send_notification("READY=1\nSTATUS=Router daemon active\n") {
        warn!("Failed to send SD_NOTIFY READY=1: {}", e);
    } else {
        info!("Sent SD_NOTIFY READY=1 to systemd");
    }
}

pub fn notify_systemd_watchdog() {
    if let Err(e) = send_notification("WATCHDOG=1\n") {
        warn!("Failed to send SD_NOTIFY WATCHDOG: {}", e);
    }
}

pub fn notify_systemd_stopping() {
    if let Err(e) = send_notification("STOPPING=1\nSTATUS=Shutting down cleanly\n") {
        warn!("Failed to send SD_NOTIFY STOPPING=1: {}", e);
    } else {
        info!("Sent SD_NOTIFY STOPPING=1 to systemd");
    }
}

#[allow(dead_code)]
pub fn notify_systemd_status(status: &str) {
    let sanitized = sanitize_value(status);
    let msg = format!("STATUS={}\n", sanitized);
    let _ = send_notification(&msg);
}

pub fn send_notification(state: &str) -> io::Result<usize> {
    if state.len() > NOTIFY_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "notification exceeds 8 MiB systemd cap",
        ));
    }
    let socket_path = match env::var("NOTIFY_SOCKET") {
        Ok(path) if !path.is_empty() => path,
        _ => return Ok(0),
    };
    send_notification_to(&socket_path, state)
}

pub fn send_notification_to(socket_path: &str, state: &str) -> io::Result<usize> {
    let addr = notify_address(socket_path).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid NOTIFY_SOCKET target '{}'", socket_path),
        )
    })?;

    let sock = socket(AddressFamily::UNIX, SocketType::DGRAM, None)?;

    let bytes = state.as_bytes();
    sendto_unix(&sock, bytes, SendFlags::empty(), &addr).map_err(|e| {
        io::Error::new(
            io::ErrorKind::Other,
            format!("sendto_unix failed for sd_notify: {}", e),
        )
    })
}
