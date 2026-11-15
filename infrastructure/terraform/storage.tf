# Object-store bucket for the CAS tier (tiered/object backends).
# S3-compatible; replace with MinIO/ceph or another provider as needed.

resource "aws_s3_bucket" "cas" {
  bucket = var.cas_bucket

  tags = {
    Name = "sandbox-cas-${var.env}"
  }
}

resource "aws_s3_bucket_versioning" "cas" {
  bucket = aws_s3_bucket.cas.id
  versioning_configuration {
    status = "Enabled"
  }
}

# Encryption at rest (SSE-S3) for the CAS blobs.
resource "aws_s3_bucket_server_side_encryption_configuration" "cas" {
  bucket = aws_s3_bucket.cas.id
  rule {
    apply_server_side_encryption_by_default {
      sse_algorithm = "AES256"
    }
    bucket_key_enabled = true
  }
}

# Lock down public access; the bucket is only reached via the IAM role.
resource "aws_s3_bucket_public_access_block" "cas" {
  bucket                  = aws_s3_bucket.cas.id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

resource "aws_s3_bucket_lifecycle_configuration" "cas" {
  bucket = aws_s3_bucket.cas.id

  rule {
    id     = "expire-incomplete-multipart-uploads"
    status = "Enabled"

    filter {}

    abort_incomplete_multipart_upload {
      days_after_initiation = 7
    }
  }
}

# Dedicated bucket for NLB access logs. The ELB service writes here; a
# separate bucket keeps log lifecycle (90-day expiry) independent of CAS.
# Only provisioned when the NLB is enabled.
resource "aws_s3_bucket" "nlb_logs" {
  count  = var.enable_lb ? 1 : 0
  bucket = "${var.cas_bucket}-nlb-logs"

  tags = {
    Name = "sandbox-nlb-logs-${var.env}"
  }
}

resource "aws_s3_bucket_public_access_block" "nlb_logs" {
  count = var.enable_lb ? 1 : 0

  bucket                  = aws_s3_bucket.nlb_logs[0].id
  block_public_acls       = true
  block_public_policy     = true
  ignore_public_acls      = true
  restrict_public_buckets = true
}

# Allow the ELB service account to write access logs.
resource "aws_s3_bucket_policy" "nlb_logs" {
  count = var.enable_lb ? 1 : 0

  bucket = aws_s3_bucket.nlb_logs[0].id

  policy = jsonencode({
    Version = "2012-10-17"
    Statement = [{
      Effect   = "Allow"
      Principal = { Service = "delivery.logs.amazonaws.com" }
      Action   = "s3:PutObject"
      Resource = "${aws_s3_bucket.nlb_logs[0].arn}/*"
      Condition = {
        StringEquals = {
          "aws:SourceAccount" = data.aws_caller_identity.current.account_id
        }
      }
    }]
  })
}

# Retain NLB access logs for 90 days.
resource "aws_s3_bucket_lifecycle_configuration" "nlb_logs" {
  count = var.enable_lb ? 1 : 0

  bucket = aws_s3_bucket.nlb_logs[0].id

  rule {
    id     = "expire-nlb-logs"
    status = "Enabled"

    filter {}

    expiration {
      days = 90
    }
  }
}
