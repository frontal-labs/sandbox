# Terraform / OpenTofu deployment for Frontal Sandbox.

Provisions one or more KVM-capable cloud hosts (AWS by default) with:
- security groups for SSH (22) and API + metrics (8080)
- an S3 bucket for the CAS tier (tiered/object backends)
- user-data that enables kvm group access

The host then runs a single `sandbox serve` process under systemd
(see `../systemd/`), installed by `../scripts/deploy.sh`. /metrics is served
on the same 8080 listener as the API (no separate metrics port).

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

- Tier 2 (QEMU) requires `/dev/kvm` for hardware acceleration; choose KVM-capable instances
  (x86_64 Nitro or aarch64 metal). cgroups v2 is default on Ubuntu 22.04+.
- The bucket name must be globally unique (S3 namespace).
- Secrets (API key digest, TLS certs, Postgres DSN) are NOT provisioned here;
  inject them via `/etc/sandbox/config.toml` or a secrets manager.
