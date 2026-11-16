# No output carries a credential.

output "name" {
  description = "The site's name."
  value       = var.name
}

output "namespace" {
  description = "The site's namespace."
  value       = local.namespace
}

output "service_name" {
  description = "The ClusterIP Service in front of the monitor."
  value       = kubernetes_service_v1.this.metadata[0].name
}

output "url" {
  description = "The site's URL: https:// when a TLS secret is named, else http://."
  value       = "${var.ingress_tls_secret_name == null ? "http" : "https"}://${var.ingress_host}/"
}

output "boards" {
  description = "The boards' names, in order."
  value       = [for b in var.boards : b.name]
}

output "board_paths" {
  description = "Each board's checkout in the pod, by name."
  value       = { for b in var.boards : b.name => "/boards/${b.name}" }
}

output "boards_with_credential" {
  description = "The names of the boards that have a credential (names only)."
  value       = sort(tolist(local.with_credential))
}

output "network_policy_enabled" {
  description = "Whether the NetworkPolicy is created."
  value       = var.network_policy_enabled
}
