# Security Policy

## kanbanr's security model (read this first)

kanbanr is deliberately small in attack surface:

- **No accounts, passwords, tokens, or network auth.** There is nothing to log into.
- **The CLI is the only writer.** It edits a local, git-backed `data/` folder on your own machine.
- **`kanbanr serve` is read-only and binds `127.0.0.1`** by default. It never mutates data; it only
  reads the folder and streams updates.
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
- The `kanbanr serve` read-only daemon (e.g. a read route exposing files outside the data folder, an
  SSE/HTTP issue).

### Out of scope

- Exposing `kanbanr serve` to a hostile network **without** a reverse proxy (this is documented as
  unsupported — the daemon has no auth by design).
- The security of your git remote / git host.
- Vulnerabilities in third-party dependencies that don't affect kanbanr's use of them (report those
  upstream; we'll bump the dependency).
