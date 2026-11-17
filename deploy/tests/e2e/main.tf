# One site, `site`, in namespace e2e, serving two boards that the e2e's
# in-cluster remote (remote.yaml) serves over plain HTTP. Each board is pulled
# every 5 s, so a push shows within 5 + 15 s (the NFR).

variable "image" {
  description = "The kanbanr image under test; e2e-kind.sh loads kanbanr:e2e into the cluster."
  type        = string
}

variable "boards" {
  description = "The boards the e2e site serves, in order."
  type = list(object({
    name                  = string
    url                   = string
    branch                = optional(string, "")
    pull_interval_seconds = optional(number, 60)
    credential_username   = optional(string, "git")
    ssh_known_hosts       = optional(string)
  }))
  default = [
    { name = "board-one", url = "http://board-remote.e2e-remote.svc.cluster.local/board.git", pull_interval_seconds = 5 },
    { name = "board-two", url = "http://board-remote.e2e-remote.svc.cluster.local/board2.git", pull_interval_seconds = 5 },
  ]
}

module "site" {
  source = "../../modules/kanbanr-site"

  name             = "site"
  namespace        = "e2e"
  create_namespace = true
  image            = var.image
  ingress_host     = "site.e2e.test"
  boards           = var.boards
}

output "service_name" {
  description = "The site's Service, which e2e-kind.sh port-forwards."
  value       = module.site.service_name
}
