/// Local-only rule, shared by setup and the daemon. The owned engine
/// and the local varlink broker are always local. Anything else must
/// live on loopback; cloud APIs are refused.
pub fn is_local_provider(kind: &str, base_url: &str) -> bool {
    match kind.to_ascii_lowercase().as_str() {
        "varlink" | "syntrop" | "runtimed" => true,
        _ => is_localhost_url(base_url),
    }
}

/// True when the URL's host is loopback (localhost, 127/8, ::1) or the
/// target is a local socket path rather than a URL at all.
pub fn is_localhost_url(url: &str) -> bool {
    if url.starts_with('/') {
        return true;
    }
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    let bare = match host.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };
    if bare.eq_ignore_ascii_case("localhost") {
        return true;
    }
    bare.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}
