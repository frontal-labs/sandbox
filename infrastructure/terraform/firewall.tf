# Firewall: lock down ingress per service, driven by explicit
# aws_security_group_rule resources (one resource per rule, so each is named,
# diffable, and independently adjustable).
#
#   22     SSH        -> var.ssh_cidr (restrict to VPN / bastion / office)
#   8080   API + /metrics -> the NLB when enable_lb, else var.ssh_cidr
#                            (metrics is served on the same 8080 listener;
#                             Prometheus scrapes /metrics on 8080 from within
#                             the VPC — no separate port is exposed)
#   egress all         -> 0.0.0.0/0 (the host must reach S3 / RDS / AMI)

# Two mutually-exclusive API ingress rules: one sourced to the NLB's security
# group when a load balancer fronts the hosts, one sourced to the operator
# CIDR otherwise. Keeping them as separate resources (rather than conditional
# fields on one rule) avoids null-value and empty-cidr edge cases in the
# provider.
resource "aws_security_group_rule" "ssh" {
  type              = "ingress"
  from_port         = 22
  to_port           = 22
  protocol          = "tcp"
  cidr_blocks       = [var.ssh_cidr]
  security_group_id = aws_security_group.sandbox.id
  description       = "SSH"
}

resource "aws_security_group_rule" "api_via_lb" {
  count = var.enable_lb ? 1 : 0

  type                     = "ingress"
  from_port                = 8080
  to_port                  = 8080
  protocol                 = "tcp"
  source_security_group_id = aws_security_group.sandbox_lb[0].id
  security_group_id        = aws_security_group.sandbox.id
  description              = "Sandbox API (NLB only)"
}

resource "aws_security_group_rule" "api_direct" {
  count = var.enable_lb ? 0 : 1

  type              = "ingress"
  from_port         = 8080
  to_port           = 8080
  protocol          = "tcp"
  cidr_blocks       = [var.ssh_cidr]
  security_group_id = aws_security_group.sandbox.id
  description       = "Sandbox API (operator CIDR; front with TLS upstream)"
}

resource "aws_security_group_rule" "egress_all" {
  type              = "egress"
  from_port         = 0
  to_port           = 0
  protocol          = "-1"
  cidr_blocks       = ["0.0.0.0/0"]
  security_group_id = aws_security_group.sandbox.id
  description       = "Allow all egress (S3, RDS, AMI, updates)"
}
