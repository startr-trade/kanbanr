# Hosting a read-only monitor

A hosted site lets a team read boards without each person running the monitor. kanbanr ships two
ways to run one, both read-only. Both serve a **list of boards**, and one board is a list of one.

- **On Kubernetes**: an OpenTofu/Terraform module in `deploy/modules/kanbanr-site`. It runs one pod
  that clones each board from its git remote and keeps it current on a schedule.
- **On one machine, with Docker Compose**: `deploy/compose/`. It runs the same pair of containers
  against remote URLs, or serves board folders that are already on the machine.

The site runs `kanbanr serve --board <path>` once per board, so its image must be **v0.8.0 or
later**. An older image exits on the unknown flag, and `tofu apply` fails while it waits for the
rollout.

The monitor has **no authentication, by design** (`ADR-0001`). On a hosted site, authentication
and TLS belong to the ingress controller or reverse proxy in front of it; see
[Authentication and limits at the ingress](#authentication-and-limits-at-the-ingress). Writes stay
off: nobody can take a decision on a hosted site. Decisions stay with each board's own writers, in
Claude Code or a local monitor.

## What the module creates

One instance of the module is one site: one host, and one clone and sync loop per board, all
feeding one server.

- **A Deployment with one replica** (`kanbanr-<name>`). Its pod holds:
  - per board, in list order, an init container `clone-<board>`. It makes a full clone of the
    board into `/boards/<board>`. Init containers run in turn, so every board is cloned before the
    monitor starts, and a site never starts with a board missing;
  - per board, a container `sync-<board>`. It fetches the board's branch every interval and
    fast-forwards the checkout **in place**, so the monitor's live refresh sees each change;
  - the container `monitor`, running `kanbanr serve --board /boards/<board>` for each board. It
    has the boards mounted **read-only**, and writes are off.

  Every container runs as user 10001, not root. Each has a read-only root file system, drops every
  capability and gets no service-account token. The sync containers use a pinned `alpine/git`
  image (git 2.54.1). The kanbanr image itself has no `git`.
- **A ClusterIP Service** on port 80, in front of the monitor's port 8080.
- **An Ingress** for an existing ingress controller, such as ingress-nginx, with one rule for the
  site's host. TLS and the load balancer belong to the controller.
- **A Secret per board that has a credential**, and none for a public board. A board's Secret is
  mounted only into that board's two git containers, never into the monitor or another board's.
- **Optionally, a NetworkPolicy** that admits only the ingress controller's pods; see
  [The NetworkPolicy](#the-networkpolicy).
- A ConfigMap holding the sync script and any SSH known-hosts lines. A change to the script
  restarts the pod.

`KANBANR_ALLOWED_HOSTS` is set to the Ingress host, so the monitor answers that host and refuses
any other. The probes call `/healthz` with `Host: localhost`, which the host check always allows.

The module takes remote URLs only. A pod cannot see a folder on your machine; to serve local board
folders, use compose's mode (b).

The cluster connection comes from the environment: `KUBE_CONFIG_PATH` names the kubeconfig file,
and `KUBE_CTX`, when set, picks its context. No cloud provider is assumed. The module uses nothing
specific to OpenTofu or Terraform, and needs version 1.8.0 or later of either. The provider is
`hashicorp/kubernetes ~> 3.5`.

### Variables

| Variable | Default | What it is |
|---|---|---|
| `name` | required | The site's name. Resources are named `kanbanr-<name>`. A lowercase DNS label of at most 22 characters. |
| `namespace` | required | The namespace for the site. |
| `create_namespace` | `false` | Create the namespace. |
| `boards` | required | The boards the site serves, in order. Each has a `name` (a unique lowercase DNS label, also its folder and the name the monitor shows), a `url`, and optionally `branch` (empty: the remote's default branch), `pull_interval_seconds` (60; a whole number from 5 to 86400), `credential_username` (`git`) and `ssh_known_hosts`. |
| `board_credentials` | `{}` | **Sensitive.** Keyed by board name: a token that can only read that repository (HTTPS remote), or a read-only deploy key's private key in OpenSSH format (SSH remote). A board with no entry is cloned anonymously. |
| `ingress_host` | required | The site's host name; also `KANBANR_ALLOWED_HOSTS`. |
| `ingress_class_name` | `null` | The IngressClass; `null` uses the cluster's default. |
| `ingress_annotations` | `{}` | Annotations for the controller, such as authentication and buffering (below). |
| `ingress_tls_secret_name` | `null` | When set, the Ingress has a `tls` entry for the host, using this Secret. |
| `image` | required | The kanbanr image on its registry, with a release tag of v0.8.0 or later. |
| `sync_image` | `alpine/git`, pinned by digest | The git image the clone and sync containers run. |
| `network_policy_enabled` | `false` | Create a NetworkPolicy that admits only the ingress controller. |
| `ingress_controller_namespace` | `"ingress-nginx"` | The controller's namespace, for the policy. |
| `ingress_controller_pod_labels` | `{"app.kubernetes.io/name" = "ingress-nginx"}` | The controller pods' labels, for the policy. |
| `board_volume_size_limit` | `"2Gi"` | The size limit of the checkouts' volume, shared by all boards. |
| `resources` | see [Resources](#resources) | Requests and limits for the monitor, and for each board's git containers. There is no CPU limit. |
| `labels` | `{}` | Extra labels on every resource. |

A board's URL is `https://…`, `http://…`, `ssh://…` or `user@host:path`. An http(s) URL with a
`user@` in it is refused: put the token in `board_credentials`, not in the URL. An SSH board needs
both a deploy key in `board_credentials` and the host's keys in `ssh_known_hosts`, or the plan
fails and names the board.

### Outputs

| Output | What it is |
|---|---|
| `name` | The site's name. |
| `namespace` | The site's namespace. |
| `service_name` | The Service in front of the monitor. |
| `url` | The site's address: `https://` when a TLS Secret is named, otherwise `http://`. |
| `boards` | The boards' names, in order. |
| `board_paths` | Each board's checkout in the pod, by name (`/boards/<name>`). |
| `boards_with_credential` | The names of the boards that have a credential. Names only. |
| `network_policy_enabled` | Whether the NetworkPolicy is created. |

No output carries a credential.

## Ordinary settings and secrets

`deploy/examples/site/` is a root module that calls the module once. Copy it, then keep its
settings in two files:

- **`terraform.tfvars`** is committed and holds the ordinary settings: the namespace, the image,
  the Ingress host, and the boards with their URLs and intervals.
- **`terraform.sensitive.auto.tfvars`** is ignored by git and holds the secrets, which are only
  `board_credentials`. Copy it from `terraform.sensitive.auto.tfvars.example`. Leave it out
  altogether when every board is public.

OpenTofu and Terraform both load `terraform.tfvars` and every `*.auto.tfvars` by themselves, so a
plain `tofu apply` (or `terraform apply`) is enough:

```
export KUBE_CONFIG_PATH=~/.kube/config
cd deploy/examples/site
tofu init
tofu apply
```

`apply` waits for the rollout, for up to 10 minutes. A bad image, an unreachable remote, an image
older than v0.8.0, or a list of boards the server refuses (two boards with one name, a folder that
is not a board, one project on two boards) fails the apply, so a site never serves half its
boards.

`deploy/tests/check_tfvars.py` fails when a `*.sensitive.auto.tfvars` or state file is tracked by
git, when a committed `.tfvars` file sets a sensitive variable, when the `.example` file holds
anything but placeholders, or when `.gitignore` stops covering those files. It runs in CI.

## Keeping state and credentials safe

A credential set in `board_credentials` ends up in the Kubernetes Secret, and also in the
OpenTofu or Terraform **state**. Treat the state as a secret:

- **Local state stays out of git.** `.gitignore` covers `*.tfstate`, `*.tfstate.*` and
  `.terraform/`, and the check above fails if one is ever tracked.
- **A remote backend must be access-controlled.** Anyone who can read the state can read every
  credential in it.
- **Encrypt the state with OpenTofu where you can.** OpenTofu has an `encryption` block for the
  root module that encrypts state and plan files. Terraform has no equivalent, so under Terraform
  the backend's own access control is all there is.
- **Every secret is marked `sensitive`**, so `plan`, `apply` and the outputs never print it.

Use the narrowest credential the host offers: a token that can only read that one repository, or a
read-only deploy key. In the pod, the credential never appears in a URL, `.git/config`, a process
argument or a log line.

## Several boards on one site

A hosted site can show more than one board. The server is started with one `--board` per board,
read-only, and serves their projects together, each from its own board. How boards are named,
what is refused, and what is and is not resolved across boards is in
[Several boards on one server](the-monitor.md#several-boards-on-one-server).

## How fresh a board is

Each board's sync fetches every `pull_interval_seconds`, 60 by default. A change pushed to the
board's remote shows on the site within one interval plus a few seconds: the checkout is
fast-forwarded in place, and open pages refresh by themselves.

What a reader sees is bounded by how often the board is **pushed**, not only by how often it is
pulled. kanbanr's default push policy, `debounce:10`, pushes once ten commits are waiting, so a
board can lag its writer's machine by up to ten commits whatever the interval. Pulling faster
than a minute rarely shows a reader anything sooner. A fetch that brings nothing is one small
round trip, about 1,440 a day per board at the default.

To see how current a board is:

- with more than one board, the home page groups the projects by board, each with a line saying
  when that board last changed;
- `/api/boards` lists each board with `head` (the commit it is at) and `head_at` (that commit's
  time). Compare `head` with the remote's tip;
- the sync container's log says what each round did. A board that has stopped following its
  remote logs `fetch failed` or `not a fast-forward` on every try; see
  [Troubleshooting](#troubleshooting).

## Authentication and limits at the ingress

The monitor checks the host name and nothing else. Add authentication at the ingress controller
through `ingress_annotations`. With ingress-nginx, basic authentication takes a Secret holding an
`htpasswd` file under the key `auth`:

```
kubectl -n kanbanr-site create secret generic site-basic-auth --from-file=auth=./htpasswd
```

```hcl
ingress_annotations = {
  "nginx.ingress.kubernetes.io/auth-type"       = "basic"
  "nginx.ingress.kubernetes.io/auth-secret"     = "site-basic-auth"
  "nginx.ingress.kubernetes.io/auth-realm"      = "kanbanr boards"
  "nginx.ingress.kubernetes.io/proxy-buffering" = "off"
}
```

- **Keep `proxy-buffering` off.** The monitor's live refresh is a stream of server-sent events. A
  buffering proxy holds those events back, and open pages stop refreshing.
- **For single sign-on**, point ingress-nginx's external authentication
  (`nginx.ingress.kubernetes.io/auth-url` and `auth-signin`) at an authenticating proxy such as
  oauth2-proxy. kanbanr needs nothing from it.
- **For a public site with real traffic**, limit the request rate at the ingress, for example
  `nginx.ingress.kubernetes.io/limit-rps = "10"`. The server already compresses its answers and
  sends cache headers keyed by the board's commit, so a reload that finds nothing new costs little.

For TLS, name a certificate Secret in `ingress_tls_secret_name`, or let a certificate manager fill
it through its own annotations.

## The NetworkPolicy

The host check stops a request that names another host, but not a caller inside the cluster that
reaches the Service directly and sends the right host name. That caller skips the ingress's
authentication. With `network_policy_enabled = true`, the module adds a NetworkPolicy that admits
traffic to the monitor's port 8080 only from pods matching both `ingress_controller_namespace` and
`ingress_controller_pod_labels`. Outgoing traffic is untouched, so the syncs still reach their
remotes, and the kubelet's probes are always allowed.

It is **off by default**, because it only works with a network plugin that enforces policies
(Calico and Cilium do; kind's default, kindnet, does not), and because the labels have to match
your controller. Left off, any pod in the cluster that can reach the Service can read the boards.

## Resources

| Container | Requests | Memory limit |
|---|---|---|
| `monitor` | 50m CPU, 64Mi | 256Mi |
| each `clone-<board>` and `sync-<board>` | 10m CPU, 32Mi | 256Mi |

There is no CPU limit. The defaults were measured on a kind cluster with a clone of kanbanr's own
board, a 15 MB bare repository (November 2026): the clone container peaked at 65 MiB and the
monitor at 37 MiB, and the pod was ready in about 30 s. Each memory limit is more than twice the
peak seen. Raise them for a much larger board, and `board_volume_size_limit` (2Gi, shared by all
boards) for many boards.

## Running it locally with compose

`deploy/compose/` runs the same site with Docker Compose, in one of two modes. Copy `.env.example`
to `.env`, which git ignores, and pick the mode with its `COMPOSE_FILE` line:

- **Mode (a), remote URLs** (`compose.yaml:compose.remote.yaml`). Copy `boards.conf.example` to
  `boards.conf`, also ignored, and list one board per line:

  ```
  # <name> <url> [<interval>] [<branch>]
  board-a https://git.example.invalid/team/board-a.git 60
  board-b https://git.example.invalid/team/board-b.git 120
  ```

  A `clone` service clones every board in full, in order, and stops at the first failure. Then
  `sync` follows each on its own interval with the same script as on Kubernetes, and the monitor
  serves them read-only. The checkouts live in a temporary volume and are cloned afresh on each
  `up`. Mode (a) takes no credential: the boards must be cloneable anonymously.
- **Mode (b), local board folders** (`compose.yaml:compose.local.yaml`). Set
  `KANBANR_BOARDS_PATH` to one board folder (the folder that holds `projects/`), or to a folder of
  board folders, and name those in `KANBANR_LOCAL_BOARDS`, comma-separated. The folder is mounted
  read-only and there is no sync container, because the monitor already watches its folders live.
  With `KANBANR_LOCAL_BOARDS` empty, the one board is named `boards`; naming the subfolders gives
  each board its folder's name. Set `KANBANR_UID` and `KANBANR_GID` to the folders' owner's
  (`id -u`, `id -g`), because the monitor reads each board's git history and git refuses a
  repository owned by another user.

The other settings in `.env`: `KANBANR_IMAGE` (v0.8.0 or later), `KANBANR_PORT` (18080) and
`KANBANR_PUBLISH_ADDRESS` (`127.0.0.1`, so the site stays on this machine), and
`KANBANR_ALLOWED_HOSTS` for the host name of a reverse proxy in front. Then:

```
cd deploy/compose
docker compose up -d
```

and open `http://localhost:18080/`. Every container runs read-only, as a non-root user, with no
capabilities.

This is not the same as `docker/docker-compose.yml` (`make docker-up`). That one is a developer's
monitor over their own board, writable; `deploy/compose/` is the read-only site.

## Drift on a hosted site

The monitor's drift panels compare a board's docs with the code they describe, and need the
project's code repository. A hosted site has only the boards, so its drift panels read
"unavailable" and say why. Serving drift there would need a pulled clone of the code repository,
named to the server by an option that is not built yet.

## Troubleshooting

Each git container logs lines of the form `<UTC time> sync: [<board>] <text>`. Read them with
`kubectl logs <pod> -c clone-<board>` or `-c sync-<board>`, or `docker compose logs sync`.

- **The pod stays in `Init`.** A board's clone is failing: its `clone-<board>` log says
  `clone failed (exit N)`, and the kubelet retries with a growing back-off. Check the URL, the
  credential, and that the cluster can reach the remote.
- **`fetch failed (exit N); keeping <sha>; next try in <n>s`.** The remote could not be reached
  or refused the credential. The board keeps being served at its last good commit. The wait
  doubles after each failure, up to 10 minutes, and returns to the interval once a fetch works.
- **`the remote's <branch> is not a fast-forward of the checkout`.** The board's history was
  rewritten, for example by a force push. The checkout is kept as it is and never reset. Restart
  the pod (`kubectl rollout restart deployment/kanbanr-<name>`) to clone it afresh.
- **`no sync round for <n>s`.** A sync loop has stalled. Its probe fails, and the kubelet restarts
  that container; the checkout survives.
- **A board's history can't be read.** `/api/boards` shows the board with `history_unreadable`
  and the reason, and a delegated decision whose grant cannot be checked says so. A repository
  owned by another user, or a shallow or partial clone, is the usual cause. The module always makes
  full clones as the monitor's user. In compose mode (b), check `KANBANR_UID` and `KANBANR_GID`.
- **`note: project '<p>' is also on board '<b>'; serving it from '<a>'`.** A project appeared on a
  second board after the server started. The first board keeps serving it, and `/api/boards` lists
  it under the second board's `shadowed`. Rename or remove one of the two; at the next start, one
  project on two boards is refused.
- **Every request answers 403.** The request's host is not the Ingress host. Behind another proxy,
  name its host in `KANBANR_ALLOWED_HOSTS` (compose), or set `ingress_host` to the name readers
  use.
