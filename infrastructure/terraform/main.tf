terraform {
  required_version = ">= 1.8"

  # Remote state so plans and applies are shared and locked across operators.
  # `terraform init` uses the local backend when S3 is not yet configured;
  # run `terraform init -migrate-state` once the bucket/key below exist.
  #
  # backend "s3" {
  #   bucket         = "frontal-sandbox-terraform-state"
  #   key            = "sandbox/terraform.tfstate"
  #   region         = "us-east-1"
  #   dynamodb_table = "frontal-sandbox-terraform-locks"
  #   encrypt        = true
  # }

  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
    tls = {
      source  = "hashicorp/tls"
      version = "~> 4.0"
    }
  }
}

variable "aws_region" {
  type        = string
  default     = "us-east-1"
  description = "AWS region to deploy into."
}

# Availability zones to spread subnets across. Leave null to auto-discover
# zones in var.aws_region (preferred, so the module works in any region).
variable "availability_zones" {
  type        = list(string)
  default     = null
  description = "AZs for subnets. Null auto-discovers from the region."
}

variable "instance_count" {
  type        = number
  default     = 1
  description = "Number of sandbox hosts."
}

# Instance type. For Tier 2 (QEMU over /dev/kvm) this MUST be a KVM-capable
# type: standard Nitro instances (m6i.large, etc.) do NOT expose /dev/kvm.
# Use a bare-metal (metal) or KVM-capable family. Default is a small metal
# instance when Tier 2 is enabled; override for cost/capacity needs.
variable "instance_type" {
  type        = string
  default     = null
  description = "Override instance type. Null picks m6i.metal-medium when Tier 2 is on, m6i.large otherwise."
}

variable "ssh_key_name" {
  type        = string
  default     = null
  description = "AWS EC2 key pair name for SSH. Null disables key-based SSH (use SSM or a bastion)."
}

# CIDR allowed to reach the SSH port. Restrict to a VPN / bastion / office
# range in production; never 0.0.0.0/0.
variable "ssh_cidr" {
  type        = string
  default     = "0.0.0.0/0"
  description = "CIDR allowed to reach port 22. Restrict this in production."
}

variable "enable_tier2" {
  type    = bool
  default = true
  description = "Whether to expose /dev/kvm for QEMU Tier 2. Forces a KVM-capable instance type."
}

# Whether to provision a load balancer in front of the hosts. When true, the
# API port is opened only to the LB and not to the world.
variable "enable_lb" {
  type    = bool
  default = false
  description = "Create an NLB in front of the sandbox hosts and open the API port only to the LB."
}

# Whether to provision a managed Postgres (RDS) for shared metadata. When
# false, hosts use the local `redb`/file backend and the DSN is not output.
variable "enable_rds" {
  type    = bool
  default = false
  description = "Provision an encrypted RDS Postgres instance for shared scheduler metadata."
}

variable "rds_instance_class" {
  type        = string
  default     = "db.t4g.small"
  description = "RDS instance class (multi-AZ capable)."
}

variable "rds_master_username" {
  type        = string
  default     = "sandbox"
  description = "RDS master (admin) username."
}

variable "rds_password" {
  type        = string
  default     = null
  sensitive   = true
  description = "RDS master password. Null generates one (read it from the output or SSM). Must be set if you want a stable password."
}

variable "cas_bucket" {
  type        = string
  default     = "sandbox-cas-production"
  description = "Globally-unique S3 bucket name for the CAS tier."
}

# Outputs live in outputs.tf.

variable "env" {
  type        = string
  default     = "prod"
  description = "Environment label, used in tags and resource names."
}

# Resolved instance type: KVM-capable when Tier 2 is on.
locals {
  effective_instance_type = coalesce(
    var.instance_type,
    var.enable_tier2 ? "m6i.metal-medium" : "m6i.large",
  )
  zones = coalesce(
    var.availability_zones,
    sort([for az in data.aws_availability_zones.available.names : az]),
  )
}

# Guard: Tier 2 needs a KVM-capable type. Standard Nitro (m5/m6i non-metal)
# instances do not expose /dev/kvm, so a bare-metal / metal or KVM-capable
# family is required. This fails at plan time rather than at runtime on a
# host without /dev/kvm. The default (m6i.metal-medium when Tier 2 is on)
# satisfies this; it only fires if an operator overrides instance_type with a
# non-metal type while leaving Tier 2 enabled.
check "tier2_needs_kvm" {
  assert {
    condition     = !var.enable_tier2 || can(regex("(?i)metal|\\bu-|\\.metal", local.effective_instance_type))
    error_message = "enable_tier2=true requires a KVM-capable instance_type (a bare-metal / metal family). Standard Nitro types like m6i.large do not expose /dev/kvm. Set instance_type to e.g. m6i.metal-medium."
  }
}

# KVM-capable instances are large; keep the root disk generous but the CAS
# cache should live on the bucket, not local disk.
variable "root_volume_size_gb" {
  type        = number
  default     = 100
  description = "GP3 root volume size in GiB."
}

data "aws_availability_zones" "available" {
  state = "available"
}

data "aws_caller_identity" "current" {}
