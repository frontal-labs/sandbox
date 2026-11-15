# Optional managed Postgres (RDS) for shared scheduler metadata. When
# enable_rds is false, hosts use the local file/redb backend and no DSN is
# output. When true, an encrypted single-AZ RDS instance is provisioned (add
# multi-AZ for higher availability) and the connection parameters are output
# for /etc/sandbox/config.toml.

resource "random_password" "rds_master" {
  count   = var.enable_rds && var.rds_password == null ? 1 : 0
  length  = 20
  special = false
}

resource "aws_db_subnet_group" "sandbox" {
  count = var.enable_rds ? 1 : 0

  name       = "sandbox-${var.env}"
  subnet_ids = aws_subnet.sandbox[*].id

  tags = {
    Name = "sandbox-${var.env}-db-sg"
  }
}

resource "aws_db_instance" "sandbox" {
  count = var.enable_rds ? 1 : 0

  identifier     = "sandbox-${var.env}"
  engine         = "postgres"
  engine_version = "15"
  instance_class = var.rds_instance_class

  allocated_storage     = 20
  max_allocated_storage = 100
  storage_encrypted     = true
  storage_type          = "gp3"
  multi_az              = true

  db_name  = "sandbox"
  username = var.rds_master_username
  password = var.rds_password != null ? var.rds_password : random_password.rds_master[0].result

  db_subnet_group_name   = aws_db_subnet_group.sandbox[0].name
  vpc_security_group_ids = [aws_security_group.sandbox.id]
  publicly_accessible    = false

  backup_retention_period = 7
  deletion_protection     = true
  skip_final_snapshot     = true

  tags = {
    Name = "sandbox-${var.env}-db"
  }
}
