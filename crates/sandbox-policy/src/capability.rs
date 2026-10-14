use sandbox_types::policy::{Capability, NetworkPolicy};
use std::net::IpAddr;

pub fn check_fs_read(policy: &NetworkPolicy, path: &str) -> bool {
    policy.check_http(path)
}

pub fn check_fs_write(policy: &NetworkPolicy, path: &str) -> bool {
    policy.check_http(path)
}

pub fn check_net_http(policy: &NetworkPolicy, host: &str) -> bool {
    policy.check_http(host)
}

pub fn check_net_tcp(policy: &NetworkPolicy, host: &str, port: u16) -> bool {
    policy.check_tcp(host, port)
}

pub fn check_capability(policy: &NetworkPolicy, cap: &Capability) -> bool {
    match cap {
        Capability::FsRead(path) => check_fs_read(policy, path),
        Capability::FsWrite(path) => check_fs_write(policy, path),
        Capability::NetHttp(host) => check_net_http(policy, host),
        Capability::NetTcp(addr) => {
            let parts: Vec<&str> = addr.split(':').collect();
            if parts.len() == 2 {
                if let Ok(port) = parts[1].parse::<u16>() {
                    return check_net_tcp(policy, parts[0], port);
                }
            }
            false
        }
        Capability::Exec(_) => true,
        Capability::Env(_) => true,
        Capability::Clock => true,
    }
}

fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            ipv4.is_loopback()
                || ipv4.is_private()
                || ipv4.is_link_local()
                || ipv4.is_broadcast()
                || ipv4.is_documentation()
        }
        IpAddr::V6(ipv6) => {
            ipv6.is_loopback() || ipv6.is_unicast_link_local() || ipv6.is_unique_local()
        }
    }
}

pub fn resolve_and_check_host(
    host: &str,
    policy: &NetworkPolicy,
) -> Result<IpAddr, sandbox_types::SandboxError> {
    let addrs: Vec<IpAddr> = std::net::ToSocketAddrs::to_socket_addrs(&format!("{}:80", host))
        .ok()
        .map(|iter| iter.map(|a| a.ip()).collect())
        .unwrap_or_default();

    for ip in &addrs {
        if is_private_ip(ip) {
            return Err(sandbox_types::SandboxError::policy_denied(format!(
                "Access to private IP address {} denied",
                ip
            )));
        }
    }

    if !policy.check_http(host) {
        return Err(sandbox_types::SandboxError::policy_denied(format!(
            "Access to host {} denied by policy",
            host
        )));
    }

    addrs.into_iter().next().ok_or_else(|| {
        sandbox_types::SandboxError::network_error(format!("Failed to resolve host: {}", host))
    })
}
