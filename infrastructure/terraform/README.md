# Terraform / OpenTofu deployment for Frontal Sandbox.

Provisions one or more KVM-capable cloud hosts (AWS by default) with:
- security groups for SSH (22), API (8080), metrics (9090, VPC-only)
- an S3 bucket for the CAS tier (tiered/object backends)
- user-data that enables kvm group access

The host then runs `sandbox-server` and `sandbox-worker` under systemd
(see `../systemd/`), installed by `../scripts/deploy.sh`.

## Usage

```bash
cd infrastructure/terraform
cp terraform.tfvars.example terraform.tfvars
# edit terraform.tfvars (region, instance type, ssh key, cas bucket)

tofu init          # or terraform init
tofu plan
tofu apply
```

## Outputs

- `security_group_id` — attach this to the host running sandbox
- `instance_private_ips` — reach the API at http://<ip>:8080

## Provider

AWS is the reference provider. Swap `aws` for `azurerm`, `google-beta`,
`openstack`, or `proxmox` by editing `main.tf` and `providers.tf`; the
security-group / bucket resources map 1:1.

## Notes

- Tier 2 (Firecracker) requires `/dev/kvm`; choose KVM-capable instances
  (x86_64 Nitro or aarch64 metal). cgroups v2 is default on Ubuntu 22.04+.
- The bucket name must be globally unique (S3 namespace).
- Secrets (API key digest, TLS certs, Postgres DSN) are NOT provisioned here;
  inject them via `/etc/sandbox/config.toml` or a secrets manager.
