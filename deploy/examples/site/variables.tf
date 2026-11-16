# Ordinary settings come from the committed terraform.tfvars; the one secret,
# board_credentials, only from the git-ignored terraform.sensitive.auto.tfvars.

variable "namespace" {
  description = "Namespace for the site."
  type        = string
}

variable "image" {
  description = "The kanbanr image on its registry with a release tag (v0.8.0 or later, which has FTR-040's --board)."
  type        = string
}

variable "ingress_host" {
  description = "The site's host; also KANBANR_ALLOWED_HOSTS."
  type        = string
}

variable "boards" {
  description = "The boards this site serves, in order; one board is a list of one."
  type = list(object({
    name                  = string
    url                   = string
    branch                = optional(string, "")
    pull_interval_seconds = optional(number, 60)
    credential_username   = optional(string, "git")
    ssh_known_hosts       = optional(string)
  }))
}

variable "board_credentials" {
  description = "Keyed by board name: a read-only token (HTTPS remote) or a read-only deploy key's private key (SSH remote). Set only in terraform.sensitive.auto.tfvars. A board with no entry is cloned anonymously."
  type        = map(string)
  sensitive   = true
  default     = {}
}
