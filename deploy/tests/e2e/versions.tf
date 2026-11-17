# The e2e root (FTR-030): run by deploy/tests/e2e-kind.sh against a throwaway
# kind cluster. Plain HCL that runs on both OpenTofu and Terraform.
terraform {
  required_version = ">= 1.8.0"

  required_providers {
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "~> 3.5"
    }
  }
}

# KUBE_CONFIG_PATH names the kind cluster's kubeconfig (tf.sh sets it from
# KUBECONFIG_FILE).
provider "kubernetes" {}
