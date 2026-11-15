provider "aws" {
  region = var.aws_region

  default_tags {
    tags = {
      ManagedBy = "terraform"
      Project   = "frontal-sandbox"
      Env       = var.env
    }
  }
}

provider "tls" {}

# IAM role scoped to read/write only the CAS bucket. The instance assumes it,
# so the worker never needs static S3 credentials in the environment.
resource "aws_iam_role" "sandbox" {
  name = "sandbox-host-${var.env}"

  assume_role_policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Action = "sts:AssumeRole"
        Effect = "Allow"
        Principal = {
          Service = "ec2.amazonaws.com"
        }
      }
    ]
  })
}

resource "aws_iam_role_policy" "sandbox_cas" {
  name = "sandbox-cas-access-${var.env}"
  role = aws_iam_role.sandbox.id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [
      {
        Effect   = "Allow"
        Action   = [
          "s3:GetObject",
          "s3:PutObject",
          "s3:DeleteObject",
          "s3:ListBucket",
          "s3:GetBucketLocation",
          "s3:AbortMultipartUpload",
          "s3:ListMultipartUploadParts",
          "s3:ListBucketMultipartUploads",
        ]
        Resource = [
          aws_s3_bucket.cas.arn,
          "${aws_s3_bucket.cas.arn}/*",
        ]
      }
    ]
  })
}

resource "aws_iam_instance_profile" "sandbox" {
  name = "sandbox-host-${var.env}"
  role = aws_iam_role.sandbox.name
}

resource "aws_instance" "sandbox" {
  count = var.instance_count

  ami           = data.aws_ami.ubuntu.id
  instance_type = local.effective_instance_type

  # KVM-capable: a bare-metal / metal or KVM-capable instance exposes /dev/kvm.
  # cgroups v2 is default on modern Ubuntu.
  key_name                    = var.ssh_key_name
  subnet_id                   = aws_subnet.sandbox[count.index % length(aws_subnet.sandbox)].id
  vpc_security_group_ids      = [aws_security_group.sandbox.id]
  iam_instance_profile        = aws_iam_instance_profile.sandbox.name
  associate_public_ip_address = var.enable_lb ? false : true

  root_block_device {
    volume_size = var.root_volume_size_gb
    volume_type = "gp3"
    iops        = 3000
    throughput  = 125
    encrypted   = true
  }

  # Idempotent bootstrap: install the release binary and start the server.
  # Re-running is safe (systemctl enable --now is idempotent; curl -fsSL fails
  # cleanly and the unit stays on the previous binary). Secrets are NOT in
  # here; they are injected via /etc/sandbox/config.toml by deploy.sh.
  user_data = <<-EOF
              #cloud-config
              packages:
                - ca-certificates
                - curl
                - tini
              runcmd:
                - usermod -aG kvm sandbox || true
                - mkdir -p /var/lib/sandbox /var/log/sandbox
                - chown -R sandbox:sandbox /var/lib/sandbox /var/log/sandbox
                - systemctl daemon-reload
                - systemctl enable --now sandbox-server
              EOF

  tags = {
    Name        = "sandbox-${var.env}-${count.index}"
    Tier2       = var.enable_tier2 ? "true" : "false"
  }

  lifecycle {
    ignore_changes = [user_data]
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

# The single security group for all sandbox hosts. All ingress/egress rules
# live in firewall.tf as explicit aws_security_group_rule resources (not inline
# blocks), so they are individually named, diffable, and easy to lock down.
resource "aws_security_group" "sandbox" {
  name_prefix = "sandbox-${var.env}"
  description = "Frontal Sandbox host"
  vpc_id      = aws_vpc.sandbox.id

  tags = {
    Name = "sandbox-${var.env}"
  }
}
