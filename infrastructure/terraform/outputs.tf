output "security_group_id" {
  description = "Security group attached to the sandbox hosts."
  value       = aws_security_group.sandbox.id
}

output "instance_private_ips" {
  description = "Private IPs of the sandbox hosts. Reach the API at http://<ip>:8080 (VPC) or via the NLB."
  value       = aws_instance.sandbox[*].private_ip
}

output "cas_bucket" {
  description = "S3 bucket name used for the CAS tier."
  value       = aws_s3_bucket.cas.bucket
}

output "cas_bucket_arn" {
  description = "S3 bucket ARN (for IAM wiring elsewhere)."
  value       = aws_s3_bucket.cas.arn
}

output "load_balancer_dns" {
  description = "NLB DNS name when enable_lb=true; otherwise null."
  value       = var.enable_lb ? aws_lb.sandbox[0].dns_name : null
}

output "rds_endpoint" {
  description = "RDS connection endpoint when enable_rds=true; otherwise null."
  value       = var.enable_rds ? aws_db_instance.sandbox[0].endpoint : null
}

output "rds_database" {
  description = "RDS database name when enable_rds=true; otherwise null."
  value       = var.enable_rds ? aws_db_instance.sandbox[0].db_name : null
}

output "rds_username" {
  description = "RDS username when enable_rds=true; otherwise null."
  value       = var.enable_rds ? aws_db_instance.sandbox[0].username : null
}

output "rds_password" {
  description = "RDS master password when enable_rds=true. Sensitive; read from SSM/Secrets Manager in prod."
  value       = var.enable_rds ? aws_db_instance.sandbox[0].password : null
  sensitive   = true
}
