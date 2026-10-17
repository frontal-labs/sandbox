resource "aws_vpc" "sandbox" {
  cidr_block = "10.0.0.0/16"
  enable_dns_hostnames = true
  enable_dns_support   = true

  tags = {
    Name        = "sandbox"
    ManagedBy   = "terraform"
  }
}

resource "aws_subnet" "sandbox" {
  count             = length(var.availability_zones)
  vpc_id            = aws_vpc.sandbox.id
  cidr_block        = cidrsubnet("10.0.0.0/16", 8, count.index)
  availability_zone = var.availability_zones[count.index]

  tags = {
    Name        = "sandbox-${count.index}"
    ManagedBy   = "terraform"
  }
}

variable "availability_zones" {
  type    = list(string)
  default = ["us-east-1a", "us-east-1b"]
}
