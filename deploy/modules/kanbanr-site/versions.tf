# kanbanr hosted site (FTR-030): one read-only monitor serving a list of
# boards, each kept current from its git remote by a sidecar.
#
# Plain HCL that runs on both OpenTofu and Terraform. 1.8.0 is the first
# OpenTofu with `mock_provider` in `tofu test` (Terraform has had it since 1.7).
terraform {
  required_version = ">= 1.8.0"

  required_providers {
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "~> 3.5"
    }
  }
}
