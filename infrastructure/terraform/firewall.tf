# Firewall: lock down ingress per service.
# 22 SSH, 8080 API, 9090 metrics (VPC-only).
# See networking.tf for the VPC/subnet definitions.

resource "aws_security_group_rule" "ssh" {
  type                     = "ingress"
  from_port                = 22
  to_port                  = 22
  protocol                 = "tcp"
  cidr_blocks              = ["0.0.0.0/0"] # operator-specified in tfvars
  security_group_id        = aws_security_group.sandbox.id
  description              = "SSH"
}

resource "aws_security_group_rule" "api" {
  type                     = "ingress"
  from_port                = 8080
  to_port                  = 8080
  protocol                 = "tcp"
  cidr_blocks              = ["0.0.0.0/0"] # TLS termination gateway in front
  security_group_id        = aws_security_group.sandbox.id
  description              = "Sandbox API (TLS-terminated upstream)"
}

resource "aws_security_group_rule" "metrics" {
  type                     = "ingress"
  from_port                = 9090
  to_port                  = 9090
  protocol                 = "tcp"
  cidr_blocks              = ["10.0.0.0/8"]
  security_group_id        = aws_security_group.sandbox.id
  description              = "Prometheus metrics (VPC only)"
}
