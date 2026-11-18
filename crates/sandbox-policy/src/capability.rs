use sandbox_types::policy::{Capability, NetworkPolicy};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};

/// No filesystem path policy exists yet, so every check is denied. The VFS and
/// its quotas are what bound filesystem access today. These functions used to
/// match the path against network host rules, which said nothing about paths.
pub fn check_fs_read(_policy: &NetworkPolicy, _path: &str) -> bool {
    false
}

/// See `check_fs_read`.
pub fn check_fs_write(_policy: &NetworkPolicy, _path: &str) -> bool {
    false
}

pub fn check_net_http(policy: &NetworkPolicy, host: &str) -> bool {
    policy.check_http(host)
}

pub fn check_net_tcp(policy: &NetworkPolicy, host: &str, port: u16) -> bool {
    policy.check_tcp(host, port)
}

/// Checks a capability against the policy. Capabilities the policy has no model
/// for (`Exec`, `Env`, `Clock`, and filesystem access) are denied until a policy
/// for them exists, so default-deny holds for every variant.
pub fn check_capability(policy: &NetworkPolicy, cap: &Capability) -> bool {
    match cap {
        Capability::FsRead(path) => check_fs_read(policy, path),
        Capability::FsWrite(path) => check_fs_write(policy, path),
        Capability::NetHttp(host) => check_net_http(policy, host),
        Capability::NetTcp(addr) => {
            let Some((host, port)) = addr.rsplit_once(':') else {
                return false;
            };
            let host = host.trim_start_matches('[').trim_end_matches(']');
            match port.parse::<u16>() {
                Ok(port) => check_net_tcp(policy, host, port),
                Err(_) => false,
            }
        }
        Capability::Exec(_) | Capability::Env(_) | Capability::Clock => false,
    }
}

/// True for addresses a sandbox must not reach: loopback, private, link-local
/// (including cloud metadata at 169.254.169.254), carrier-grade NAT, benchmark,
/// documentation, multicast, reserved, and unspecified ranges, and IPv6 forms
/// that embed such an IPv4 address.
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => is_private_v6(v6),
    }
}

fn is_private_v4(ip: &Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 0 // 0.0.0.0/8, "this network"
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || (o[0] == 100 && (o[1] & 0xC0) == 64) // 100.64.0.0/10, carrier-grade NAT
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24, IETF protocol assignments
        || (o[0] == 198 && (o[1] & 0xFE) == 18) // 198.18.0.0/15, benchmarking
        || ip.is_documentation()
        || ip.is_multicast()
        || o[0] >= 240 // 240.0.0.0/4 reserved, and broadcast
}

fn is_private_v6(ip: &Ipv6Addr) -> bool {
    // IPv4-mapped (::ffff:a.b.c.d) and IPv4-compatible (::a.b.c.d) addresses
    // are judged by the IPv4 address they carry.
    if let Some(v4) = ip.to_ipv4() {
        return is_private_v4(&v4);
    }
    let s = ip.segments();
    // NAT64 (64:ff9b::/96) and 6to4 (2002::/16) embed an IPv4 address.
    if s[0] == 0x0064 && s[1] == 0xff9b && s[2..6].iter().all(|&x| x == 0) {
        let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
        return is_private_v4(&v4);
    }
    if s[0] == 0x2002 {
        let v4 = Ipv4Addr::new((s[1] >> 8) as u8, s[1] as u8, (s[2] >> 8) as u8, s[2] as u8);
        return is_private_v4(&v4);
    }
    ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (s[0] & 0xFFC0) == 0xFE80 // fe80::/10 link-local
        || (s[0] & 0xFE00) == 0xFC00 // fc00::/7 unique local
        || (s[0] & 0xFFC0) == 0xFEC0 // fec0::/10 site-local, deprecated
        || (s[0] == 0x2001 && s[1] == 0x0db8) // 2001:db8::/32 documentation
}

/// Checks `host` against the policy, then resolves it and rejects it if any
/// address is private. The policy is checked first so a denied host causes no
/// DNS lookup, which would otherwise be a channel for leaking data in names.
///
/// This resolves once. The caller must connect to the returned address and not
/// resolve the name again, or a second answer could point somewhere private.
pub fn resolve_and_check_host(
    host: &str,
    policy: &NetworkPolicy,
) -> Result<IpAddr, sandbox_types::SandboxError> {
    if !policy.check_http(host) {
        return Err(sandbox_types::SandboxError::policy_denied(format!(
            "Access to host {} denied by policy",
            host
        )));
    }

    let literal = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>();
    let addrs: Vec<IpAddr> = match literal {
        Ok(ip) => vec![ip],
        Err(_) => (host, 80u16)
            .to_socket_addrs()
            .map(|iter| iter.map(|a| a.ip()).collect())
            .unwrap_or_default(),
    };

    for ip in &addrs {
        if is_private_ip(ip) {
            return Err(sandbox_types::SandboxError::policy_denied(format!(
                "Access to private IP address {} denied",
                ip
            )));
        }
    }

    addrs.into_iter().next().ok_or_else(|| {
        sandbox_types::SandboxError::network_error(format!("Failed to resolve host: {}", host))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_types::policy::NetworkPolicyRule;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("test address parses")
    }

    #[test]
    fn test_private_ranges_are_blocked() {
        for addr in [
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "100.64.0.1",
            "100.127.255.254",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.8",
            "192.168.1.1",
            "198.18.0.1",
            "198.19.255.255",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fe80::1",
            "fc00::1",
            "fd00:ec2::254",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:10.0.0.1",
            "64:ff9b::a00:1",
            "2002:7f00:1::1",
            "2001:db8::1",
        ] {
            assert!(is_private_ip(&ip(addr)), "{addr} should be blocked");
        }
    }

    #[test]
    fn test_public_addresses_are_allowed() {
        for addr in [
            "8.8.8.8",
            "1.1.1.1",
            "100.63.255.255",
            "100.128.0.1",
            "198.17.0.1",
            "198.20.0.1",
            "172.15.0.1",
            "172.32.0.1",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
        ] {
            assert!(!is_private_ip(&ip(addr)), "{addr} should be allowed");
        }
    }

    #[test]
    fn test_capabilities_without_a_policy_model_are_denied() {
        let policy = NetworkPolicy::allow_all();
        for cap in [
            Capability::fs_read("/etc/passwd"),
            Capability::fs_write("/tmp/x"),
            Capability::exec("sh"),
            Capability::env("HOME"),
            Capability::clock(),
        ] {
            assert!(!check_capability(&policy, &cap), "{cap:?} should be denied");
        }
    }

    #[test]
    fn test_network_capabilities_follow_the_policy() {
        let mut policy = NetworkPolicy::default();
        policy.add_rule(NetworkPolicyRule::allow_http("example.com"));
        policy.add_rule(NetworkPolicyRule::allow_tcp("db.example.com", vec![5432]));
        assert!(check_capability(
            &policy,
            &Capability::net_http("example.com")
        ));
        assert!(!check_capability(
            &policy,
            &Capability::net_http("other.com")
        ));
        assert!(check_capability(
            &policy,
            &Capability::net_tcp("db.example.com:5432")
        ));
        assert!(!check_capability(
            &policy,
            &Capability::net_tcp("db.example.com:22")
        ));
    }

    #[test]
    fn test_malformed_tcp_addresses_are_denied() {
        let policy = NetworkPolicy::allow_all();
        for addr in ["", "host", "host:", "host:notaport", "host:70000"] {
            assert!(
                !check_capability(&policy, &Capability::net_tcp(addr)),
                "{addr:?} should be denied"
            );
        }
    }

    #[test]
    fn test_ipv6_tcp_address_is_parsed() {
        let mut policy = NetworkPolicy::default();
        policy.add_rule(NetworkPolicyRule::allow_tcp(
            "2606:4700:4700::1111",
            vec![443],
        ));
        assert!(check_capability(
            &policy,
            &Capability::net_tcp("[2606:4700:4700::1111]:443")
        ));
    }

    #[test]
    fn test_denied_host_is_rejected_before_any_lookup() {
        let policy = NetworkPolicy::default();
        // `.invalid` never resolves. A policy denial, not a resolution error,
        // shows the policy ran first.
        let err = resolve_and_check_host("secret-data.attacker.invalid", &policy)
            .expect_err("denied by policy");
        assert!(
            err.to_string().contains("denied by policy"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_private_literal_is_rejected_even_when_policy_allows_all() {
        let policy = NetworkPolicy::allow_all();
        for host in ["127.0.0.1", "169.254.169.254", "::1", "[::ffff:10.0.0.1]"] {
            let err = resolve_and_check_host(host, &policy).expect_err("private address");
            assert!(
                err.to_string().contains("private IP"),
                "{host}: unexpected error: {err}"
            );
        }
    }

    #[test]
    fn test_public_literal_resolves_without_dns() {
        let policy = NetworkPolicy::allow_all();
        assert_eq!(
            resolve_and_check_host("8.8.8.8", &policy).expect("public literal"),
            ip("8.8.8.8")
        );
    }
}
