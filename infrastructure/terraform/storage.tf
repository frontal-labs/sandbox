# Object-store bucket for the CAS tier (TieredStore / ObjectStore backends).
# S3-compatible; replace with MinIO/ceph or another provider as needed.

resource "aws_s3_bucket" "cas" {
  bucket = var.cas_bucket

  tags = {
    Name        = "sandbox-cas"
    ManagedBy   = "terraform"
  }
}

resource "aws_s3_bucket_versioning" "cas" {
  bucket = aws_s3_bucket.cas.id
  versioning_configuration {
    status = "Enabled"
  }
}

resource "aws_s3_bucket_lifecycle_configuration" "cas" {
  bucket = aws_s3_bucket.cas.id

  rule {
    id     = "expire-incomplete-multipart-uploads"
    status = "Enabled"

    abort_incomplete_multipart_upload_days = 7
  }
}

variable "cas_bucket" {
  type    = string
  default = "sandbox-cas"
}
