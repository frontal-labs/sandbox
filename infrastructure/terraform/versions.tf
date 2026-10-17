# versions.lock — run `terraform providers lock -platform=all` after editing
# required_providers in main.tf.

provider "registry.opensource.hashicorp.com/hashicorp/aws" {
  version     = "5.x"
  hashes = [
    "h1:...",
  ]
}

provider "registry.opensource.hashicorp.com/hashicorp/tls" {
  version     = "4.x"
  hashes = [
    "h1:...",
  ]
}
