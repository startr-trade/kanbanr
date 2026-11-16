terraform {
  required_version = ">= 1.8.0"

  required_providers {
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "~> 3.5"
    }
  }
}

# The cluster connection comes from the environment: KUBE_CONFIG_PATH names
# the kubeconfig file and KUBE_CTX, when set, picks its context. No cloud
# provider is assumed.
provider "kubernetes" {}
