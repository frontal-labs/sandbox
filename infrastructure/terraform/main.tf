terraform {
  required_version = ">= 1.8"
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
  type    = string
  default = "us-east-1"
}

variable "instance_type" {
  type    = string
  default = "m6i.large"
  description = "Must be a KVM-capable instance (x86_64 or aarch64 bare metal / Nitro)."
}

variable "instance_count" {
  type    = number
  default = 1
}

variable "ssh_key_name" {
  type    = string
  default = null
}

variable "enable_tier2" {
  type    = bool
  default = true
  description = "Whether to attach /dev/kvm access for Firecracker Tier 2."
}

output "security_group_id" {
  value = aws_security_group.sandbox.id
}

output "instance_private_ips" {
  value = aws_instance.sandbox[*].private_ip
}
