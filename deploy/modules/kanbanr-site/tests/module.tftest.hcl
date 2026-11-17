# The module's plan, against a mock provider (FTR-030 R-1, R-5, R-7, R-9).
# `tofu test` / `terraform test` print `run "<name>"... pass` for each run.

mock_provider "kubernetes" {}

variables {
  name         = "site"
  namespace    = "kanbanr-site"
  ingress_host = "boards.example.invalid"
  image        = "kanbanr:test"
  boards = [
    { name = "board-a", url = "https://git.example.invalid/team/board-a.git" },
  ]
}

run "a_list_of_one_makes_one_deployment_and_one_sync_per_board" {
  command = plan

  assert {
    condition     = kubernetes_deployment_v1.this.spec[0].replicas == "1"
    error_message = "the site must run one replica"
  }

  assert {
    condition     = [for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].init_container : c.name] == ["clone-board-a"]
    error_message = "one board must make one clone init container"
  }

  assert {
    condition     = [for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container : c.name] == ["sync-board-a", "monitor"]
    error_message = "one board must make one sync container beside the monitor"
  }

  assert {
    condition     = one([for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container : c.args if c.name == "monitor"]) == tolist(["kanbanr", "serve", "--board", "/boards/board-a"])
    error_message = "the monitor must run kanbanr serve with one --board per board"
  }

  assert {
    condition     = length([for e in one([for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container : c.env if c.name == "monitor"]) : e if e.name == "KANBANR_DATA_DIR"]) == 0
    error_message = "the monitor must not set KANBANR_DATA_DIR"
  }

  assert {
    condition     = kubernetes_service_v1.this.spec[0].type == "ClusterIP" && kubernetes_service_v1.this.spec[0].port[0].port == 80
    error_message = "the site must have a ClusterIP Service on port 80"
  }

  assert {
    condition     = kubernetes_ingress_v1.this.spec[0].rule[0].host == "boards.example.invalid"
    error_message = "the Ingress must answer the site's host"
  }

  assert {
    condition     = tolist(output.boards) == tolist(["board-a"]) && length(output.board_paths) == 1 && output.board_paths["board-a"] == "/boards/board-a"
    error_message = "the outputs must list the one board"
  }
}

run "two_boards_make_two_clones_two_syncs_and_one_monitor" {
  command = plan

  variables {
    # Not in sorted order, so list order is what is checked.
    boards = [
      { name = "zeta", url = "https://git.example.invalid/team/zeta.git" },
      { name = "alpha", url = "https://git.example.invalid/team/alpha.git", pull_interval_seconds = 120 },
    ]
  }

  assert {
    condition     = kubernetes_deployment_v1.this.spec[0].replicas == "1"
    error_message = "the site must run one replica"
  }

  assert {
    condition     = [for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].init_container : c.name] == ["clone-zeta", "clone-alpha"]
    error_message = "each board must make a clone init container, in list order"
  }

  assert {
    condition     = [for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container : c.name] == ["sync-zeta", "sync-alpha", "monitor"]
    error_message = "each board must make a sync container, in list order, and there must be one monitor"
  }

  assert {
    condition     = one([for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container : c.args if c.name == "monitor"]) == tolist(["kanbanr", "serve", "--board", "/boards/zeta", "--board", "/boards/alpha"])
    error_message = "the monitor must get one --board per board, in list order"
  }

  assert {
    condition     = one([for e in one([for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container : c.env if c.name == "sync-alpha"]) : e.value if e.name == "PULL_INTERVAL"]) == "120"
    error_message = "each board's sync must use its own pull interval"
  }

  assert {
    condition     = one([for e in one([for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container : c.env if c.name == "sync-zeta"]) : e.value if e.name == "BOARD_DIR"]) == "/boards/zeta"
    error_message = "each board's checkout must be /boards/<name>"
  }

  assert {
    condition     = tolist(output.boards) == tolist(["zeta", "alpha"])
    error_message = "the boards output must keep list order"
  }
}

run "the_monitor_is_non_root_read_only_and_probes_as_localhost" {
  command = plan

  assert {
    condition = (
      kubernetes_deployment_v1.this.spec[0].template[0].spec[0].security_context[0].run_as_non_root == true &&
      kubernetes_deployment_v1.this.spec[0].template[0].spec[0].security_context[0].run_as_user == "10001" &&
      kubernetes_deployment_v1.this.spec[0].template[0].spec[0].security_context[0].run_as_group == "10001" &&
      kubernetes_deployment_v1.this.spec[0].template[0].spec[0].security_context[0].fs_group == "10001"
    )
    error_message = "the pod must run as the non-root user 10001"
  }

  assert {
    condition     = kubernetes_deployment_v1.this.spec[0].template[0].spec[0].automount_service_account_token == false
    error_message = "the pod must not mount a service account token"
  }

  assert {
    condition = alltrue([for c in concat(
      kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container,
      kubernetes_deployment_v1.this.spec[0].template[0].spec[0].init_container,
      ) : (
      c.security_context[0].read_only_root_filesystem == true &&
      c.security_context[0].allow_privilege_escalation == false &&
      tolist(c.security_context[0].capabilities[0].drop) == tolist(["ALL"])
    )])
    error_message = "every container must have a read-only root, no privilege escalation and drop ALL"
  }

  assert {
    condition     = one([for m in one([for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container : c.volume_mount if c.name == "monitor"]) : m.read_only if m.mount_path == "/boards"]) == true
    error_message = "the monitor must mount /boards read-only"
  }

  assert {
    condition = alltrue([for p in [
      one([for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container : c.startup_probe[0] if c.name == "monitor"]),
      one([for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container : c.readiness_probe[0] if c.name == "monitor"]),
      one([for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container : c.liveness_probe[0] if c.name == "monitor"]),
      ] : (
      p.http_get[0].path == "/healthz" &&
      p.http_get[0].http_header[0].name == "Host" &&
      p.http_get[0].http_header[0].value == "localhost"
    )])
    error_message = "every monitor probe must call /healthz with Host: localhost"
  }

  assert {
    condition     = one([for e in one([for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container : c.env if c.name == "monitor"]) : e.value if e.name == "KANBANR_ALLOWED_HOSTS"]) == "boards.example.invalid"
    error_message = "KANBANR_ALLOWED_HOSTS must be the Ingress host"
  }
}

run "anonymous_boards_create_no_secret" {
  command = plan

  variables {
    boards = [
      { name = "board-a", url = "https://git.example.invalid/team/board-a.git" },
      { name = "board-b", url = "https://git.example.invalid/team/board-b.git" },
    ]
  }

  assert {
    condition     = length(kubernetes_secret_v1.git) == 0
    error_message = "anonymous boards must create no Secret"
  }

  assert {
    condition     = length([for v in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].volume : v.name if startswith(v.name, "cred-")]) == 0
    error_message = "anonymous boards must mount no credential"
  }

  assert {
    condition     = alltrue([for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container : alltrue([for e in c.env : e.value == "none" if e.name == "CREDENTIAL_KIND"])])
    error_message = "anonymous boards must sync with CREDENTIAL_KIND=none"
  }

  assert {
    condition     = length(output.boards_with_credential) == 0
    error_message = "no board has a credential"
  }

  assert {
    condition     = length(kubernetes_network_policy_v1.this) == 0
    error_message = "the NetworkPolicy is off by default"
  }
}

run "a_token_makes_one_secret_mounted_only_for_its_board" {
  command = plan

  variables {
    boards = [
      { name = "board-a", url = "https://git.example.invalid/team/board-a.git" },
      { name = "board-b", url = "https://git.example.invalid/team/board-b.git" },
    ]
    board_credentials = { board-a = "test-token" }
  }

  assert {
    condition     = tolist(keys(kubernetes_secret_v1.git)) == tolist(["board-a"])
    error_message = "a token for board-a must make one Secret"
  }

  assert {
    condition     = kubernetes_secret_v1.git["board-a"].type == "Opaque" && kubernetes_secret_v1.git["board-a"].metadata[0].name == "kanbanr-site-board-a-git"
    error_message = "an HTTPS board's Secret is Opaque and named kanbanr-<name>-<board>-git"
  }

  assert {
    condition     = tolist(nonsensitive(keys(kubernetes_secret_v1.git["board-a"].data))) == tolist(["token"])
    error_message = "an HTTPS board's Secret holds the key token"
  }

  assert {
    condition = sort(flatten([for c in concat(
      kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container,
      kubernetes_deployment_v1.this.spec[0].template[0].spec[0].init_container,
    ) : [for m in c.volume_mount : c.name if m.mount_path == "/credential"]])) == tolist(["clone-board-a", "sync-board-a"])
    error_message = "the credential must be mounted only in board-a's clone and sync containers"
  }

  assert {
    condition     = one([for e in one([for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container : c.env if c.name == "sync-board-a"]) : e.value if e.name == "CREDENTIAL_KIND"]) == "token"
    error_message = "board-a must sync with CREDENTIAL_KIND=token"
  }

  assert {
    condition     = one([for e in one([for c in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].container : c.env if c.name == "sync-board-b"]) : e.value if e.name == "CREDENTIAL_KIND"]) == "none"
    error_message = "board-b must sync with CREDENTIAL_KIND=none"
  }

  assert {
    condition     = output.boards_with_credential == tolist(["board-a"])
    error_message = "the output names board-a only"
  }
}

run "an_ssh_remote_needs_a_key_and_known_hosts" {
  command = plan

  variables {
    boards = [
      { name = "board-a", url = "git@git.example.invalid:team/board-a.git" },
    ]
    board_credentials = { board-a = "test-key" }
  }

  expect_failures = [
    kubernetes_deployment_v1.this,
  ]
}

run "the_network_policy_admits_only_the_ingress_controller" {
  command = plan

  variables {
    network_policy_enabled = true
  }

  assert {
    condition     = length(kubernetes_network_policy_v1.this) == 1
    error_message = "enabling the policy must create one NetworkPolicy"
  }

  assert {
    condition     = tolist(kubernetes_network_policy_v1.this[0].spec[0].policy_types) == tolist(["Ingress"])
    error_message = "the policy must be of type Ingress only"
  }

  assert {
    condition = (
      length(kubernetes_network_policy_v1.this[0].spec[0].ingress) == 1 &&
      length(kubernetes_network_policy_v1.this[0].spec[0].ingress[0].from) == 1 &&
      kubernetes_network_policy_v1.this[0].spec[0].ingress[0].from[0].namespace_selector[0].match_labels == tomap({ "kubernetes.io/metadata.name" = "ingress-nginx" }) &&
      kubernetes_network_policy_v1.this[0].spec[0].ingress[0].from[0].pod_selector[0].match_labels == tomap({ "app.kubernetes.io/name" = "ingress-nginx" })
    )
    error_message = "the policy must admit one peer with both the namespace and the pod selector"
  }

  assert {
    condition     = kubernetes_network_policy_v1.this[0].spec[0].ingress[0].ports[0].port == "8080" && kubernetes_network_policy_v1.this[0].spec[0].ingress[0].ports[0].protocol == "TCP"
    error_message = "the policy must admit 8080/TCP"
  }

  assert {
    condition     = kubernetes_network_policy_v1.this[0].spec[0].pod_selector[0].match_labels == tomap({ "app.kubernetes.io/name" = "kanbanr-site", "app.kubernetes.io/instance" = "site" })
    error_message = "the policy must select the site's pods"
  }
}

run "a_named_pull_secret_is_given_to_the_pod" {
  command = plan

  variables {
    image_pull_secret = "registry-creds"
  }

  assert {
    condition = (
      length(kubernetes_deployment_v1.this.spec[0].template[0].spec[0].image_pull_secrets) == 1 &&
      [for s in kubernetes_deployment_v1.this.spec[0].template[0].spec[0].image_pull_secrets : s.name] == ["registry-creds"]
    )
    error_message = "the pod must name the one pull secret it was given"
  }
}

run "no_pull_secret_is_named_by_default" {
  command = plan

  assert {
    condition     = length(kubernetes_deployment_v1.this.spec[0].template[0].spec[0].image_pull_secrets) == 0
    error_message = "the pod must name no pull secret when none is given"
  }
}
