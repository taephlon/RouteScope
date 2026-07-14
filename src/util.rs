//! Utility helpers for RouteScope

/// Returns true if the IP string represents a private / loopback address.
/// Handles:
///   - 127.0.0.0/8    (loopback)
///   - 10.0.0.0/8     (private)
///   - 172.16.0.0/12  (private)
///   - 192.168.0.0/16 (private)
///   - ::1            (IPv6 loopback)
///   - fc00::/7       (IPv6 ULA)
///   - fe80::/10      (IPv6 link-local)
pub fn is_private_ip(ip_str: &str) -> bool {
    if ip_str == "127.0.0.1" || ip_str == "::1" {
        return true;
    }
    if ip_str.starts_with("10.") || ip_str.starts_with("192.168.") {
        return true;
    }
    // 172.16.0.0/12 → 172.16.x.x – 172.31.x.x
    if let Some(rest) = ip_str.strip_prefix("172.") {
        if let Some(second_octet_str) = rest.split('.').next() {
            if let Ok(second) = second_octet_str.parse::<u8>() {
                if (16..=31).contains(&second) {
                    return true;
                }
            }
        }
    }
    // IPv6 ULA (fc00::/7)
    if ip_str.starts_with("fc") || ip_str.starts_with("fd") {
        return true;
    }
    // IPv6 link-local (fe80::/10)
    if ip_str.starts_with("fe80") {
        return true;
    }
    false
}

/// Format a duration in milliseconds to a human-readable string
pub fn format_ms(ms: f64) -> String {
    if ms < 1.0 {
        format!("{:.0} µs", ms * 1000.0)
    } else if ms < 1000.0 {
        format!("{:.2} ms", ms)
    } else {
        format!("{:.2} s", ms / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loopback() {
        assert!(is_private_ip("127.0.0.1"));
        assert!(is_private_ip("::1"));
    }

    #[test]
    fn test_rfc1918() {
        assert!(is_private_ip("10.0.0.1"));
        assert!(is_private_ip("10.255.255.255"));
        assert!(is_private_ip("192.168.1.1"));
        assert!(is_private_ip("192.168.0.100"));
        assert!(is_private_ip("172.16.0.1"));
        assert!(is_private_ip("172.31.255.255"));
    }

    #[test]
    fn test_public() {
        assert!(!is_private_ip("8.8.8.8"));
        assert!(!is_private_ip("1.1.1.1"));
        assert!(!is_private_ip("103.5.6.7"));
        assert!(!is_private_ip("172.32.0.1")); // just outside 172.16/12
    }

    #[test]
    fn test_format_ms() {
        assert!(format_ms(0.5).contains("µs"));
        assert!(format_ms(15.0).contains("ms"));
        assert!(format_ms(1500.0).contains("s"));
    }
}
