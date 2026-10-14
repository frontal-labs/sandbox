use sandbox_types::policy::{NetworkPolicy, NetworkPolicyRule};

pub fn create_default_policy() -> NetworkPolicy {
    NetworkPolicy::default()
}

pub fn create_allow_all_policy() -> NetworkPolicy {
    NetworkPolicy::allow_all()
}

pub fn create_github_policy() -> NetworkPolicy {
    let mut policy = NetworkPolicy::default();
    policy.add_rule(NetworkPolicyRule::allow_http("github.com"));
    policy.add_rule(NetworkPolicyRule::allow_http("api.github.com"));
    policy.add_rule(NetworkPolicyRule::allow_http("raw.githubusercontent.com"));
    policy.add_rule(NetworkPolicyRule::allow_http(
        "objects.githubusercontent.com",
    ));
    policy
}

pub fn create_npm_policy() -> NetworkPolicy {
    let mut policy = NetworkPolicy::default();
    policy.add_rule(NetworkPolicyRule::allow_http("registry.npmjs.org"));
    policy.add_rule(NetworkPolicyRule::allow_http("api.npmjs.org"));
    policy
}

pub fn create_pypi_policy() -> NetworkPolicy {
    let mut policy = NetworkPolicy::default();
    policy.add_rule(NetworkPolicyRule::allow_http("pypi.org"));
    policy.add_rule(NetworkPolicyRule::allow_http("files.pythonhosted.org"));
    policy
}

pub fn create_docker_policy() -> NetworkPolicy {
    let mut policy = NetworkPolicy::default();
    policy.add_rule(NetworkPolicyRule::allow_http("registry-1.docker.io"));
    policy.add_rule(NetworkPolicyRule::allow_http("auth.docker.io"));
    policy
}
