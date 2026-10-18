//! Host classification for links and browser navigation: this machine and private networks are never
//! reached on the AI's behalf.

/// `localhost` or a loopback address (`127.0.0.0/8`, `::1`).
pub fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Hosts the AI must not reach: this machine, private networks and single-label (intranet) names.
pub fn is_private_host(host: &str) -> bool {
    let h = host.trim_matches(['[', ']']).to_ascii_lowercase();
    if h == "localhost"
        || h.ends_with(".local")
        || h.ends_with(".internal")
        || h.ends_with(".localhost")
        || !h.contains('.') && h.parse::<std::net::IpAddr>().is_err()
    {
        return true;
    }
    match h.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast()
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            ip.is_loopback() || ip.is_unspecified() || (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_classification() {
        for h in [
            "localhost",
            "127.0.0.1",
            "[::1]",
            "10.1.2.3",
            "192.168.0.9",
            "169.254.1.1",
            "172.16.0.1",
            "intranet",
            "printer.local",
            "db.internal",
            "fd00::1",
            "fe80::1",
            "0.0.0.0",
        ] {
            assert!(is_private_host(h), "{h}");
        }
        for h in ["example.com", "8.8.8.8", "learn.microsoft.com", "2606:4700::1111"] {
            assert!(!is_private_host(h), "{h}");
        }
        assert!(is_loopback_host("localhost") && is_loopback_host("127.0.0.2") && is_loopback_host("[::1]"));
        assert!(!is_loopback_host("192.168.1.1") && !is_loopback_host("example.com"));
    }
}
