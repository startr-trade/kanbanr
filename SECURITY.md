# Security Policy

## kanbanr's security model (read this first)

kanbanr is deliberately small in attack surface:

- **No accounts, passwords, tokens, or network auth.** There is nothing to log into.
- **The CLI is the only writer.** It edits a local, git-backed `data/` folder on your own machine.
- **`kanbanr serve` binds `127.0.0.1` and is read-only** by default: it reads the folder and streams
  updates. Started with `--allow-writes` (or through `kanbanr review --ui`) the monitor can also take
  decisions, and those writes are limited to the monitor's own page. At each start the monitor makes a
  new random token and gives it only to the page it serves; a write is accepted only with that token
  and, when the browser sends an `Origin`, only from the monitor's own origin. It sends no
  cross-origin headers, and it answers a request only for the loopback names, the address it is bound
  to and the hosts listed in `KANBANR_ALLOWED_HOSTS` (comma-separated `name[:port]`, for a reverse
  proxy). Anything else gets a 403 and changes nothing.
- **Exposing the monitor beyond localhost is the operator's responsibility.** If you put `serve`
  behind a network, front it with a reverse proxy that you control (TLS + authentication). kanbanr
  itself does no access control.
- **Sharing/centralization is delegated to your git remote** (e.g. GitHub). Access control to your
  data is your git host's access control — not kanbanr's.

Because of this model, there are no application credentials to leak. The trust boundary is your
**local machine**, your **git remote's permissions**, and any **reverse proxy** you place in front
of the monitor.

> Note: older builds had a multi-user auth layer and a `data/security.yaml`. That model has been
> removed. `security.yaml` is git-ignored defensively; if one exists in your checkout it is inert —
> delete it.

## What a release carries

Every release is built from the locked dependencies (`Cargo.lock` and `web/package-lock.json`), and its assets are listed in `SHA256SUMS` beside two CycloneDX SBOMs, one for the Rust dependencies and one for the monitor's npm dependencies. Each asset and the container image also has a build attestation naming the repository, commit and workflow that produced it, unless the release notes say it carries none. To check a download, run `gh attestation verify <file> -R startr-trade/kanbanr`; [Installation](https://kanbanr.startr.trade/getting-started/installation.html#checking-a-download) has the full steps.

## Supported versions

This is a personal project on a `0.x` line; only the **latest released version** is supported.
Please reproduce issues against the newest tag (or `main`) before reporting.

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

- Preferred: open a **private security advisory** via GitHub →
  `https://github.com/startr-trade/kanbanr/security/advisories/new`
  (Security tab → "Report a vulnerability").
- Or email **kanbanr-oss-support@startr.trade** with details and, if possible, a minimal reproduction.

Please include: affected version/commit, steps to reproduce, impact, and any suggested fix.

### What to expect

- Acknowledgement on a **best-effort** basis (this is a single-maintainer project — no guaranteed
  response time).
- A fix or mitigation in a subsequent release, with credit to you in the changelog unless you prefer
  to remain anonymous.

### In scope

- The `kanbanr` CLI and `kanbanr-core` engine (e.g. path traversal in doc/asset handling, unsafe
  git operations, data-loss bugs on sync/commit).
- The `kanbanr serve` daemon (e.g. a read route exposing files outside the data folder, a write
  accepted from outside the monitor's own page, an SSE/HTTP issue).

### Out of scope

- Exposing `kanbanr serve` to a hostile network **without** a reverse proxy (this is documented as
  unsupported — the daemon has no auth by design).
- The security of your git remote / git host.
- Vulnerabilities in third-party dependencies that don't affect kanbanr's use of them (report those
  upstream; we'll bump the dependency).
