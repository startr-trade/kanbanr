# One instance of this module is one site: one Deployment, one host, and one
# clone and sync loop per board, all feeding one `kanbanr serve` (FTR-030).

variable "name" {
  description = "Site name; resources are kanbanr-<name>."
  type        = string

  validation {
    condition     = can(regex("^[a-z]([-a-z0-9]{0,20}[a-z0-9])?$", var.name))
    error_message = "name must be a lowercase DNS label of at most 22 characters, starting with a letter (^[a-z]([-a-z0-9]{0,20}[a-z0-9])?$)."
  }
}

variable "namespace" {
  description = "Namespace for the site."
  type        = string
}

variable "create_namespace" {
  description = "Create the namespace."
  type        = bool
  default     = false
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

  validation {
    condition     = length(var.boards) >= 1
    error_message = "boards must list at least one board."
  }

  validation {
    condition     = length(distinct([for b in var.boards : b.name])) == length(var.boards)
    error_message = "board names must be unique."
  }

  validation {
    condition     = alltrue([for b in var.boards : can(regex("^[a-z]([-a-z0-9]{0,20}[a-z0-9])?$", b.name))])
    error_message = "each board name must be a lowercase DNS label of at most 22 characters, starting with a letter (^[a-z]([-a-z0-9]{0,20}[a-z0-9])?$)."
  }

  validation {
    condition     = alltrue([for b in var.boards : can(regex("^(https://|http://|ssh://|[^@/:\\s]+@[^:\\s]+:)", b.url))])
    error_message = "each board url must be https://…, http://…, ssh://… or user@host:path."
  }

  validation {
    condition     = alltrue([for b in var.boards : !can(regex("^https?://[^/]*@", b.url))])
    error_message = "a board's http(s) url must not carry a user@: put the token in board_credentials, not in the URL."
  }

  validation {
    condition = alltrue([for b in var.boards :
      b.pull_interval_seconds == floor(b.pull_interval_seconds) &&
      b.pull_interval_seconds >= 5 && b.pull_interval_seconds <= 86400
    ])
    error_message = "each board's pull_interval_seconds must be a whole number from 5 to 86400."
  }
}

variable "board_credentials" {
  description = "Keyed by board name. The value is either a token that can only read that repository (HTTPS remote) or a read-only deploy key's private key in OpenSSH format (SSH remote). Set only in terraform.sensitive.auto.tfvars. A board with no entry is cloned anonymously."
  type        = map(string)
  sensitive   = true
  default     = {}
}

variable "ingress_host" {
  description = "The site's host; also KANBANR_ALLOWED_HOSTS."
  type        = string

  validation {
    condition     = can(regex("^[a-z0-9]([-a-z0-9]{0,61}[a-z0-9])?(\\.[a-z0-9]([-a-z0-9]{0,61}[a-z0-9])?)*$", var.ingress_host)) && length(var.ingress_host) <= 253
    error_message = "ingress_host must be a lowercase DNS name."
  }
}

variable "ingress_class_name" {
  description = "IngressClass; null means the cluster default."
  type        = string
  default     = null
}

variable "ingress_annotations" {
  description = "Controller annotations (auth, buffering: see the docs page)."
  type        = map(string)
  default     = {}
}

variable "ingress_tls_secret_name" {
  description = "When set, the Ingress has a tls entry for the host."
  type        = string
  default     = null
}

variable "image" {
  description = "The kanbanr image on its registry with a release tag (v0.8.0 or later, which has FTR-040's --board), an ordinary setting in terraform.tfvars."
  type        = string
}

variable "image_pull_secret" {
  description = "The name of a Secret of type kubernetes.io/dockerconfigjson that already exists in the namespace, for a private registry. The module never creates it."
  type        = string
  default     = null
}

variable "sync_image" {
  description = "The pinned git image."
  type        = string
  # Multi-arch index digest, read 16 Nov 2026.
  # `docker run --rm --entrypoint git <digest> --version` (17 Nov 2026):
  # git version 2.54.1
  default = "docker.io/alpine/git@sha256:2306e2b05ee0ef99f5d25b0779f645b24ae9f91ba187107503592fcdfe78da3a"
}

variable "network_policy_enabled" {
  description = "Create a NetworkPolicy admitting only the ingress controller."
  type        = bool
  default     = false
}

variable "ingress_controller_namespace" {
  description = "For the policy."
  type        = string
  default     = "ingress-nginx"
}

variable "ingress_controller_pod_labels" {
  description = "For the policy."
  type        = map(string)
  default     = { "app.kubernetes.io/name" = "ingress-nginx" }
}

variable "board_volume_size_limit" {
  description = "sizeLimit of the checkouts' emptyDir, shared by all boards."
  type        = string
  default     = "2Gi"
}

variable "resources" {
  description = "Requests and limits. The sync values apply to each board's containers. There is no CPU limit."
  type = object({
    monitor = optional(object({
      cpu_request    = optional(string, "50m")
      memory_request = optional(string, "64Mi")
      memory_limit   = optional(string, "256Mi")
    }), {})
    sync = optional(object({
      cpu_request    = optional(string, "10m")
      memory_request = optional(string, "32Mi")
      memory_limit   = optional(string, "256Mi")
    }), {})
  })
  default = {}
}

variable "labels" {
  description = "Extra labels."
  type        = map(string)
  default     = {}
}
