resource "kubernetes_namespace_v1" "this" {
  count = var.create_namespace ? 1 : 0

  metadata {
    name   = var.namespace
    labels = local.labels
  }
}

# The one sync script, and each SSH board's known hosts.
resource "kubernetes_config_map_v1" "sync" {
  metadata {
    name      = "${local.resource_name}-sync"
    namespace = local.namespace
    labels    = local.labels
  }

  data = merge(
    { "sync.sh" = local.sync_script },
    { for b in local.board_list : "known_hosts-${b.name}" => b.ssh_known_hosts if b.ssh_known_hosts != null },
  )
}

# One Secret per board that has a credential: a token for an HTTPS remote, a
# deploy key for an SSH one. A board with no credential is cloned anonymously.
resource "kubernetes_secret_v1" "git" {
  for_each = local.with_credential

  metadata {
    name      = "${local.resource_name}-${each.key}-git"
    namespace = local.namespace
    labels    = local.labels
  }

  type = local.boards[each.key].ssh ? "kubernetes.io/ssh-auth" : "Opaque"

  data = {
    (local.boards[each.key].ssh ? "ssh-privatekey" : "token") = var.board_credentials[each.key]
  }
}

resource "kubernetes_deployment_v1" "this" {
  metadata {
    name      = local.resource_name
    namespace = local.namespace
    labels    = local.labels
  }

  wait_for_rollout = true

  timeouts {
    create = "10m"
  }

  lifecycle {
    precondition {
      condition     = length(local.ssh_boards_incomplete) == 0
      error_message = "board ${join(", ", local.ssh_boards_incomplete)}: an SSH remote needs a read-only deploy key in board_credentials and the host's keys in ssh_known_hosts"
    }
  }

  spec {
    # One replica: the charter rules out several servers over one data folder.
    # Each pod has its own checkouts, so a rollout overlap is two readers of
    # two folders, not two servers over one.
    replicas = 1

    strategy {
      type = "RollingUpdate"
      rolling_update {
        max_surge       = "1"
        max_unavailable = "0"
      }
    }

    selector {
      match_labels = local.selector_labels
    }

    template {
      metadata {
        labels = local.labels
        annotations = {
          "kanbanr-site/sync-sha" = sha256(local.sync_script)
        }
      }

      spec {
        automount_service_account_token = false
        enable_service_links            = false

        # Every container runs as 10001, so each checkout's owner is the
        # monitor's user (libgit2's owner check).
        security_context {
          run_as_non_root = true
          run_as_user     = 10001
          run_as_group    = 10001
          fs_group        = 10001
          seccomp_profile {
            type = "RuntimeDefault"
          }
        }

        volume {
          name = "boards"
          empty_dir {
            size_limit = var.board_volume_size_limit
          }
        }

        volume {
          name = "monitor-tmp"
          empty_dir {
            medium     = "Memory"
            size_limit = "16Mi"
          }
        }

        dynamic "volume" {
          for_each = local.board_list
          content {
            name = "git-tmp-${volume.value.name}"
            empty_dir {
              medium     = "Memory"
              size_limit = "64Mi"
            }
          }
        }

        volume {
          name = "scripts"
          config_map {
            name         = kubernetes_config_map_v1.sync.metadata[0].name
            default_mode = "0555"
          }
        }

        # Group-readable: secret files are owned by root and read by group 10001.
        dynamic "volume" {
          for_each = [for b in local.board_list : b if contains(local.with_credential, b.name)]
          content {
            name = "cred-${volume.value.name}"
            secret {
              secret_name  = kubernetes_secret_v1.git[volume.value.name].metadata[0].name
              default_mode = "0440"
            }
          }
        }

        # Init containers run in order, so every board is cloned before the
        # monitor starts.
        dynamic "init_container" {
          for_each = local.board_list
          content {
            name    = "clone-${init_container.value.name}"
            image   = var.sync_image
            command = ["/bin/sh", "/scripts/sync.sh", "clone"]

            dynamic "env" {
              for_each = local.board_env[init_container.value.name]
              content {
                name  = env.key
                value = env.value
              }
            }

            volume_mount {
              name       = "boards"
              mount_path = "/boards"
            }
            volume_mount {
              name       = "scripts"
              mount_path = "/scripts"
              read_only  = true
            }
            volume_mount {
              name       = "git-tmp-${init_container.value.name}"
              mount_path = "/tmp"
            }
            dynamic "volume_mount" {
              for_each = contains(local.with_credential, init_container.value.name) ? [init_container.value.name] : []
              content {
                name       = "cred-${volume_mount.value}"
                mount_path = "/credential"
                read_only  = true
              }
            }

            security_context {
              allow_privilege_escalation = false
              read_only_root_filesystem  = true
              capabilities {
                drop = ["ALL"]
              }
            }

            resources {
              requests = {
                cpu    = var.resources.sync.cpu_request
                memory = var.resources.sync.memory_request
              }
              limits = {
                memory = var.resources.sync.memory_limit
              }
            }
          }
        }

        dynamic "container" {
          for_each = local.board_list
          content {
            name    = "sync-${container.value.name}"
            image   = var.sync_image
            command = ["/bin/sh", "/scripts/sync.sh", "follow"]

            dynamic "env" {
              for_each = local.board_env[container.value.name]
              content {
                name  = env.key
                value = env.value
              }
            }

            volume_mount {
              name       = "boards"
              mount_path = "/boards"
            }
            volume_mount {
              name       = "scripts"
              mount_path = "/scripts"
              read_only  = true
            }
            volume_mount {
              name       = "git-tmp-${container.value.name}"
              mount_path = "/tmp"
            }
            dynamic "volume_mount" {
              for_each = contains(local.with_credential, container.value.name) ? [container.value.name] : []
              content {
                name       = "cred-${volume_mount.value}"
                mount_path = "/credential"
                read_only  = true
              }
            }

            liveness_probe {
              exec {
                command = ["/bin/sh", "/scripts/sync.sh", "alive"]
              }
              period_seconds    = 30
              failure_threshold = 3
            }

            security_context {
              allow_privilege_escalation = false
              read_only_root_filesystem  = true
              capabilities {
                drop = ["ALL"]
              }
            }

            resources {
              requests = {
                cpu    = var.resources.sync.cpu_request
                memory = var.resources.sync.memory_request
              }
              limits = {
                memory = var.resources.sync.memory_limit
              }
            }
          }
        }

        container {
          name  = "monitor"
          image = var.image
          # No KANBANR_DATA_DIR: --board overrides the image's /data (FTR-040 I-3).
          args = local.monitor_args

          env {
            name  = "KANBANR_BIND"
            value = "0.0.0.0:8080"
          }
          env {
            name  = "KANBANR_ALLOWED_HOSTS"
            value = var.ingress_host
          }

          port {
            name           = "http"
            container_port = 8080
          }

          volume_mount {
            name       = "boards"
            mount_path = "/boards"
            read_only  = true
          }
          volume_mount {
            name       = "monitor-tmp"
            mount_path = "/tmp"
          }

          # The probes send Host: localhost, which the host check allows.
          startup_probe {
            http_get {
              path = "/healthz"
              port = "http"
              http_header {
                name  = "Host"
                value = "localhost"
              }
            }
            period_seconds    = 2
            failure_threshold = 30
          }

          readiness_probe {
            http_get {
              path = "/healthz"
              port = "http"
              http_header {
                name  = "Host"
                value = "localhost"
              }
            }
            period_seconds    = 10
            timeout_seconds   = 3
            failure_threshold = 3
          }

          liveness_probe {
            http_get {
              path = "/healthz"
              port = "http"
              http_header {
                name  = "Host"
                value = "localhost"
              }
            }
            period_seconds    = 20
            timeout_seconds   = 3
            failure_threshold = 3
          }

          security_context {
            allow_privilege_escalation = false
            read_only_root_filesystem  = true
            capabilities {
              drop = ["ALL"]
            }
          }

          resources {
            requests = {
              cpu    = var.resources.monitor.cpu_request
              memory = var.resources.monitor.memory_request
            }
            limits = {
              memory = var.resources.monitor.memory_limit
            }
          }
        }
      }
    }
  }
}

resource "kubernetes_service_v1" "this" {
  metadata {
    name      = local.resource_name
    namespace = local.namespace
    labels    = local.labels
  }

  spec {
    type     = "ClusterIP"
    selector = local.selector_labels

    port {
      name        = "http"
      port        = 80
      target_port = "http"
    }
  }
}

# TLS and the load balancer belong to the existing ingress controller.
resource "kubernetes_ingress_v1" "this" {
  metadata {
    name        = local.resource_name
    namespace   = local.namespace
    labels      = local.labels
    annotations = var.ingress_annotations
  }

  spec {
    ingress_class_name = var.ingress_class_name

    rule {
      host = var.ingress_host
      http {
        path {
          path      = "/"
          path_type = "Prefix"
          backend {
            service {
              name = kubernetes_service_v1.this.metadata[0].name
              port {
                number = 80
              }
            }
          }
        }
      }
    }

    dynamic "tls" {
      for_each = var.ingress_tls_secret_name == null ? [] : [var.ingress_tls_secret_name]
      content {
        hosts       = [var.ingress_host]
        secret_name = tls.value
      }
    }
  }
}

# Admits only the ingress controller's pods. Egress is untouched, so the syncs
# reach their remotes; kubelet probes are always allowed.
resource "kubernetes_network_policy_v1" "this" {
  count = var.network_policy_enabled ? 1 : 0

  metadata {
    name      = local.resource_name
    namespace = local.namespace
    labels    = local.labels
  }

  spec {
    pod_selector {
      match_labels = local.selector_labels
    }

    policy_types = ["Ingress"]

    ingress {
      from {
        namespace_selector {
          match_labels = { "kubernetes.io/metadata.name" = var.ingress_controller_namespace }
        }
        pod_selector {
          match_labels = var.ingress_controller_pod_labels
        }
      }

      ports {
        port     = "8080"
        protocol = "TCP"
      }
    }
  }
}
