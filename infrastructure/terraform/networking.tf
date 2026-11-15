resource "aws_vpc" "sandbox" {
  cidr_block           = "10.0.0.0/16"
  enable_dns_hostnames = true
  enable_dns_support   = true

  tags = {
    Name = "sandbox-${var.env}"
  }
}

# Subnets spread across the (possibly auto-discovered) availability zones.
resource "aws_subnet" "sandbox" {
  count = length(local.zones)

  vpc_id            = aws_vpc.sandbox.id
  cidr_block        = cidrsubnet("10.0.0.0/16", 8, count.index)
  availability_zone = local.zones[count.index]

  tags = {
    Name = "sandbox-${var.env}-${count.index}"
  }
}

# Internet gateway + public route so instances can egress to S3 / RDS / AMI.
# The hosts still keep their API port locked down by the security group.
resource "aws_internet_gateway" "sandbox" {
  vpc_id = aws_vpc.sandbox.id

  tags = {
    Name = "sandbox-${var.env}-igw"
  }
}

resource "aws_route_table" "sandbox" {
  vpc_id = aws_vpc.sandbox.id

  tags = {
    Name = "sandbox-${var.env}"
  }
}

resource "aws_route" "sandbox_internet" {
  route_table_id         = aws_route_table.sandbox.id
  destination_cidr_block = "0.0.0.0/0"
  gateway_id             = aws_internet_gateway.sandbox.id
}

resource "aws_route_table_association" "sandbox" {
  count = length(aws_subnet.sandbox)

  subnet_id      = aws_subnet.sandbox[count.index].id
  route_table_id = aws_route_table.sandbox.id
}

# Optional NLB in front of the sandbox hosts. When enable_lb is set, the API
# port (8080) is opened only to this LB's security group, not the world. TLS
# termination is the LB's job (the server itself has no TLS); put a
# TLS listener / certificate in front of or on this NLB per your network.
resource "aws_lb" "sandbox" {
  count = var.enable_lb ? 1 : 0

  name               = "sandbox-${var.env}"
  internal           = false
  load_balancer_type = "network"
  subnets            = aws_subnet.sandbox[*].id

  tags = {
    Name = "sandbox-${var.env}-nlb"
  }
}

resource "aws_lb_listener" "sandbox_api" {
  count = var.enable_lb ? 1 : 0

  load_balancer_arn = aws_lb.sandbox[0].arn
  port              = 8080
  protocol          = "TCP"

  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.sandbox_api[0].arn
  }
}

resource "aws_lb_target_group" "sandbox_api" {
  count = var.enable_lb ? 1 : 0

  name     = "sandbox-${var.env}-api"
  port     = 8080
  protocol = "TCP"
  vpc_id   = aws_vpc.sandbox.id

  health_check {
    enabled = true
    port    = 8080
    protocol = "TCP"
  }

  tags = {
    Name = "sandbox-${var.env}-api-tg"
  }
}

resource "aws_lb_target_group_attachment" "sandbox_api" {
  count = var.enable_lb ? var.instance_count : 0

  target_group_arn = aws_lb_target_group.sandbox_api[0].arn
  target_id        = aws_instance.sandbox[count.index].id
  port             = 8080
}

# A dedicated SG for the NLB so the "api" ingress rule can source-allow only
# the LB and nothing else.
resource "aws_security_group" "sandbox_lb" {
  count = var.enable_lb ? 1 : 0

  name_prefix = "sandbox-${var.env}-lb"
  description = "NLB fronting the sandbox hosts"
  vpc_id      = aws_vpc.sandbox.id

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = {
    Name = "sandbox-${var.env}-lb"
  }
}
