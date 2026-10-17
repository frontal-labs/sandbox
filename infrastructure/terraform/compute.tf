provider "aws" {
  region = var.aws_region
}

provider "tls" {}

resource "aws_instance" "sandbox" {
  count         = var.instance_count
  ami           = data.aws_ami.ubuntu.id
  instance_type = var.instance_type

  # KVM-capable: x86_64 Nitro or aarch64 metal instances expose /dev/kvm.
  # cgroups v2 is default on modern Ubuntu.
  key_name = var.ssh_key_name

  vpc_security_group_ids = [aws_security_group.sandbox.id]

  user_data = <<-EOF
              #cloud-config
              packages:
                - ca-certificates
                - curl
                - tini
              runcmd:
                - usermod -aG kvm sandbox || true
                - systemctl enable --now sandbox-server sandbox-scheduler || true
              EOF

  root_block_device {
    volume_size = 100
    volume_type = "gp3"
    encrypted   = true
  }

  ephemeral_block_device {
    device_name = "/dev/nvme1n1"
  }

  tags = {
    Name        = "sandbox-${count.index}"
    ManagedBy   = "terraform"
    Tier2       = var.enable_tier2
  }
}

data "aws_ami" "ubuntu" {
  most_recent = true
  owners      = ["099720109477"] # Canonical

  filter {
    name   = "name"
    values = ["ubuntu-jammy-22.04-amd64-server-*"]
  }
}

resource "aws_security_group" "sandbox" {
  name_prefix = "sandbox"
  description = "Frontal Sandbox host"

  ingress {
    from_port   = 22
    to_port     = 22
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"] # narrow this per your network policy
  }

  ingress {
    from_port   = 8080
    to_port     = 8080
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"] # fronted by TLS + auth gateway in prod
  }

  ingress {
    from_port   = 9090
    to_port     = 9090
    protocol    = "tcp"
    cidr_blocks = ["10.0.0.0/8"] # metrics only from within the VPC
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = {
    Name        = "sandbox"
    ManagedBy   = "terraform"
  }
}
