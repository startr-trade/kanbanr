locals {
  # Each board by name, with whether its remote is SSH and its checkout folder.
  boards = { for b in var.boards : b.name => merge(b, {
    ssh = can(regex("^(ssh://|[^@/:\\s]+@[^:\\s]+:)", b.url))
    dir = "/boards/${b.name}"
  }) }

  # The same boards as a list, in var.boards order. A map iterates by sorted
  # key, so the per-board dynamic blocks iterate over this to keep list order.
  board_list = [for b in var.boards : local.boards[b.name]]

  # The set of names with a credential is not secret, and for_each keys
  # cannot be sensitive.
  with_credential = toset([for k in keys(local.boards) : k if nonsensitive(contains(keys(var.board_credentials), k))])

  # SSH boards missing a key or known hosts; the Deployment's precondition.
  ssh_boards_incomplete = [for b in local.board_list : b.name if b.ssh && !(contains(local.with_credential, b.name) && b.ssh_known_hosts != null)]

  # The same form for a list of one and a list of many, in list order.
  monitor_args = concat(["kanbanr", "serve"], flatten([for b in var.boards : ["--board", "/boards/${b.name}"]]))

  selector_labels = {
    "app.kubernetes.io/name"     = "kanbanr-site"
    "app.kubernetes.io/instance" = var.name
  }

  labels = merge(local.selector_labels, {
    "app.kubernetes.io/managed-by" = "kanbanr-deploy"
  }, var.labels)

  resource_name = "kanbanr-${var.name}"
  namespace     = var.create_namespace ? kubernetes_namespace_v1.this[0].metadata[0].name : var.namespace

  sync_script = file("${path.module}/files/sync.sh")

  # Each board's environment for its clone and sync containers.
  board_env = { for b in local.board_list : b.name => {
    BOARD_NAME          = b.name
    BOARD_URL           = b.url
    BOARD_BRANCH        = b.branch
    BOARD_DIR           = b.dir
    PULL_INTERVAL       = tostring(b.pull_interval_seconds)
    MAX_BACKOFF         = "600"
    FETCH_TIMEOUT       = "120"
    CLONE_TIMEOUT       = "900"
    HOME                = "/tmp/home"
    HEARTBEAT           = "/tmp/sync-heartbeat"
    GIT_TERMINAL_PROMPT = "0"
    GIT_CONFIG_NOSYSTEM = "1"
    CREDENTIAL_KIND     = contains(local.with_credential, b.name) ? (b.ssh ? "ssh" : "token") : "none"
    CREDENTIAL_USERNAME = b.credential_username
    CREDENTIAL_DIR      = "/credential"
    KNOWN_HOSTS_FILE    = "/scripts/known_hosts-${b.name}"
  } }
}
