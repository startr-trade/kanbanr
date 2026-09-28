# kanbanr — Open-Sourcing & Release Guide

> A concrete, checklist-style path to publishing kanbanr publicly and keeping it maintainable —
> sized for a **personal open-source project** (one maintainer, friendly to contributors), not a
> foundation-governed one. Pair with [ROADMAP.md](ROADMAP.md) and the board's own assessment
> (`kanbanr doc show notes/assessment.md`).
>
> Status today (updated): the workspace is now relicensed **MIT OR Apache-2.0** with
> `LICENSE-MIT` + `LICENSE-APACHE` files; `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`,
> `CHANGELOG.md`, `THIRD_PARTY.md`, GitHub **issue/PR templates**, **Dependabot**, a **CI** workflow
> and a **release** workflow all exist. What's left is mostly **your** work: reserve names, create
> accounts/tokens, fill two placeholders, add screenshots, and cut the first tagged release. Checked
> boxes below mark what's already in the repo.
>
> **Placeholders: filled.** The GitHub owner is `startr-trade` (README, this file, templates,
> workflows, manifests), and the contact address in `CODE_OF_CONDUCT.md` and `SECURITY.md` is
> `kanbanr-oss-support@startr.trade`. The copyright line in `LICENSE-MIT` / `LICENSE-APACHE` and
> the `authors` field in `api/Cargo.toml` deliberately read *the kanbanr authors* — change it only
> if you want a personal name on the licence.
>
>   ```bash
>   # Forking this under a different owner? One pass replaces every reference:
>   grep -rl 'startr-trade' . --exclude-dir=.git --exclude-dir=node_modules --exclude-dir=target \
>     | xargs sed -i 's/startr-trade/YOUR_GITHUB_OWNER/g'
>   ```

## Accounts, keys & credentials you'll need (start here if this is your first OSS project)

Open-sourcing kanbanr means publishing to a few different services. Each needs an **account** and
usually a **token** — a long secret string that proves you're allowed to publish. You create the
token once on the service's website, then either paste it into your terminal (for a one-off manual
publish) or store it as a **GitHub Actions secret** (so the automated release workflow can publish
for you). **Never** paste a token into a file that gets committed.

**The golden rules (these are the "measures" to take):**
- **Turn on 2FA** (two-factor auth) on every account below. crates.io and npm now *require* it to publish.
- **Use a password manager** for accounts + tokens.
- **Scope tokens narrowly and give them an expiry.** A publish token should only be able to publish.
- **Prefer "Trusted Publishing" where offered** (crates.io and npm support it): instead of a
  long-lived token, GitHub Actions proves its identity to the registry over OIDC at publish time, so
  there's no secret to leak. You set it up on the registry's website by naming your GitHub repo +
  workflow. This is the modern best practice — use it if you can; the token method below is the fallback.
- **Store automation tokens only in** GitHub → your repo → **Settings → Secrets and variables →
  Actions → New repository secret.** Workflows read them as `${{ secrets.NAME }}`; they're masked in
  logs and never shared with pull requests from forks.
- **Rotate (regenerate) a token immediately if it ever leaks**, and treat anything that touched a
  commit as already leaked.

### What each destination needs

| Destination | What it's for | Account | Credential to create | Where it lives |
|---|---|---|---|---|
| **GitHub** | Hosts the repo, runs CI, stores release binaries, serves the plugin marketplace | github.com (free) + **2FA** | an **SSH key** for `git push`; CI's `GITHUB_TOKEN` is automatic | SSH key on your laptop (`~/.ssh`); `GITHUB_TOKEN` is injected into Actions — nothing to store |
| **crates.io** | Publishes the `kanbanr` CLI so `cargo install kanbanr` works | crates.io (log in with GitHub) + **2FA** | an **API token**, *or* set up **Trusted Publishing** | `CARGO_REGISTRY_TOKEN` secret, **or** none (Trusted Publishing) |
| **GHCR** (GitHub Container Registry) | Publishes the Docker image so `docker run ghcr.io/startr-trade/kanbanr` works | your GitHub account | **none** — Actions uses the automatic `GITHUB_TOKEN` (`packages: write`) | nothing to store; locally use a PAT with `write:packages` |
| **Claude Code plugin marketplace** | Lets users `claude plugin install kanbanr` | your GitHub account | **none** — it's just your public git repo | nothing — users add the repo URL |
| **VS Code Marketplace** *(only when you ship the extension — `editor/vscode/`)* | Publishes the VS Code extension | **Azure DevOps** account (free) + a **publisher** at marketplace.visualstudio.com/manage | a **Personal Access Token** scoped *Marketplace → Manage* | `VSCE_PAT` secret |
| **Open VSX** *(optional, open-source VS Code registry)* | Same extension for VSCodium/Cursor/etc. | open-vsx.org (log in with GitHub) | an access token | `OVSX_TOKEN` secret |
| **npm** *(probably NOT needed)* | Only if you ever publish the web UI as a reusable package — kanbanr bundles it into the binary, so you likely just **reserve the name** | npmjs.com + **2FA** | an **Automation token** | `NPM_TOKEN` secret |

> You don't need every row. For a **first release** the essentials are **GitHub** (always),
> **crates.io** (for `cargo install`), and **GHCR** (for `docker run`). The Claude plugin needs
> nothing beyond a public repo. VS Code / Open VSX / npm can come later.

### Step-by-step: creating each credential

**GitHub SSH key (to push code)** — on your laptop:
```bash
ssh-keygen -t ed25519 -C "you@example.com"   # press Enter for defaults; set a passphrase
cat ~/.ssh/id_ed25519.pub                     # copy this line
```
Paste it at GitHub → Settings → **SSH and GPG keys** → New SSH key. Test: `ssh -T git@github.com`.

**crates.io API token (to publish the CLI):**
1. Sign in at crates.io with GitHub; enable 2FA.
2. Account Settings → **API Tokens** → New Token. Name it `kanbanr-release`; scope to
   **publish-update** (plus **publish-new** for the very first publish); set an expiry.
3. Manual publish: `cargo login` then paste it (stored in `~/.cargo/credentials.toml` — never commit).
4. Automated publish: add it as the `CARGO_REGISTRY_TOKEN` secret **and** set repo variable
   `PUBLISH_CRATES=true` (the release workflow's crates job is guarded on that). **Better:** skip the
   token and set up **Trusted Publishing** on crates.io (your crate → Settings → Trusted Publishing →
   add `startr-trade/kanbanr` + the `release.yml` workflow), then delete the token line.
5. ⚠️ **A published version is permanent** — you can `cargo yank` a bad version but never delete or
   reuse it. Double-check before `cargo publish`.

**GHCR (Docker image):** nothing to create — the release workflow logs in with the built-in
`GITHUB_TOKEN`. To push from your laptop instead, make a GitHub **Personal Access Token** (Settings →
Developer settings → PATs) with `write:packages`, then
`echo $PAT | docker login ghcr.io -u startr-trade --password-stdin`.

**VS Code Marketplace PAT (when you publish the extension):**
1. Create a free **Azure DevOps** account at dev.azure.com (Microsoft runs the VS Code Marketplace on
   it — counter-intuitive but required).
2. Create a **publisher** at https://marketplace.visualstudio.com/manage.
3. Azure DevOps → User settings → **Personal Access Tokens** → New: Organization *All accessible
   organizations*, Scopes *Custom defined → Marketplace → Manage*, set an expiry.
4. Publish: `npx vsce publish -p <PAT>`, or store it as the `VSCE_PAT` secret and let CI run it.
   Optional Open VSX: account at open-vsx.org → token → `npx ovsx publish -p <OVSX_TOKEN>`.

**Where the automation secrets go:** GitHub repo → **Settings → Secrets and variables → Actions →
New repository secret**: `CARGO_REGISTRY_TOKEN`, `VSCE_PAT`, etc. (and the `PUBLISH_CRATES` *variable*
under the same page's "Variables" tab). The workflows reference them as `${{ secrets.NAME }}` /
`${{ vars.NAME }}`; they never appear in logs.

---

## 0. Before you make the repo public — a safety sweep

Do this first; making a repo public (and pushing its history) is hard to undo.

kanbanr has **no accounts, passwords, or tokens of its own** (single-writer CLI + read-only viewer —
see [DESIGN.md](DESIGN.md)), so there are no app secrets to leak. The real risks are your **own
content** and your **publish credentials**:

- [x] **Nothing of the board ships with the source.** kanbanr's own board lives in a sibling
      repository beside this one (`../<name>.kanbanr`), which is the layout the tool recommends to everyone
      (FEAT-041): a board inside a checkout is one `git add -A` away from being committed, and a
      sibling cannot be. Publishing it, if you ever want to, is a separate `git remote add` on that
      repository — not a decision about this one.
- [x] **The legacy `security.yaml` is gone** — an inert leftover from the removed auth model
      (ADR-0001). It was never tracked (`git ls-files | grep security.yaml` prints nothing in both
      repositories) and the file itself has been deleted.
- [ ] **Check git remotes for embedded credentials.** A remote like
      `https://user:token@host/repo.git` leaks the token — use SSH remotes; run `git remote -v` to confirm.
- [ ] **Grep the tree *and history*** for anything sensitive:
      `git grep -niE "secret|token|password|api[_-]?key|@.*\.(com|net)" $(git rev-list --all)`.
- [ ] If anything sensitive was ever committed, **rewrite history**
      (`git filter-repo --invert-paths --path <file>`) or, simplest, **start a fresh repo from a
      clean checkout** with no prior history — then **rotate** the leaked credential (treat anything
      that touched a commit as burned).

## 1. Licensing & legal

- [x] **Dual-licensed MIT OR Apache-2.0** — done: [`LICENSE-MIT`](../LICENSE-MIT) +
      [`LICENSE-APACHE`](../LICENSE-APACHE) added and `license = "MIT OR Apache-2.0"` set in
      `api/Cargo.toml` (workspace). *(Replace the copyright holder `the kanbanr authors` with your
      legal name/handle if you prefer; if you'd rather stay MIT-only, delete `LICENSE-APACHE` and set
      `license = "MIT"`.)*
- [ ] (Optional) Add `# SPDX-License-Identifier: MIT OR Apache-2.0` headers where convenient.
- [ ] **Name check (do before announcing):** confirm `kanbanr` is free on **crates.io** (and **npm**
      if you'll reserve it), plus the GitHub `startr-trade/kanbanr` repo and a domain if you want one.
      Reserve early — `cargo publish` of a `0.0.0` placeholder claims the crate name.
- [x] **Third-party notices** documented in [`THIRD_PARTY.md`](../THIRD_PARTY.md) (vendored
      **libgit2** GPL-2.0-WITH-linking-exception + **OpenSSL** Apache-2.0, statically linked — fine
      for MIT/Apache distribution). Still **to do:** run `cargo deny check licenses` (and add it to
      CI) to verify nothing incompatible slipped in.

## 2. Repo presentation (the "front page")

- [ ] **README** polish: one-line pitch, a screenshot or short GIF of the live monitor, the
      60-second quickstart, the architecture diagram (reuse [DESIGN.md](DESIGN.md)), and a clear
      "is this for me?" (personal, git-backed, Claude-driven).
- [ ] Badges: CI status, license, latest release, crates.io version.
- [ ] A `docs/` index linking USER_GUIDE, DESIGN, ROADMAP, this file.
- [ ] Capture screenshots in `docs/src/images/` (`make screenshots`) (board, feature page, status page, milestones, docs).
      A reproducible **Selenium-Grid-in-Docker screenshot tool** lives in
      [`../tools/screenshots/`](../tools/screenshots/) — `make screenshots` regenerates them all
      against a locally-running `kanbanr serve`. (No more "login" screen to capture — the monitor has
      no auth.)

## 3. Contributor & community files

- [x] **[`CONTRIBUTING.md`](../CONTRIBUTING.md)** — build/test/lint commands, the design constraints
      (CLI is the only writer), code style, PR flow, inbound=outbound licensing.
- [x] **[`CODE_OF_CONDUCT.md`](../CODE_OF_CONDUCT.md)** — Contributor Covenant 2.1. *(Fill in the
      contact method placeholder it ships with.)*
- [x] **[`SECURITY.md`](../SECURITY.md)** — private reporting (GitHub advisory / email) and the
      actual security model: **no accounts/auth**, the `serve` daemon is read-only on 127.0.0.1, and
      exposing it is the operator's job (reverse proxy + TLS). Reports go to a GitHub private
      security advisory or **kanbanr-oss-support@startr.trade**.
- [x] **Issue + PR templates** under [`../.github/`](../.github/) (`ISSUE_TEMPLATE/bug_report.yml`,
      `feature_request.yml`, `config.yml`, `PULL_REQUEST_TEMPLATE.md`).
- [x] **Support promise** stated as "personal project, best-effort, no SLA" (in CONTRIBUTING/SECURITY).
- [ ] (Optional) Enable **GitHub Discussions** for Q&A (the issue `config.yml` links to it).

## 4. CI (GitHub Actions)

- [x] **[`ci.yml`](../.github/workflows/ci.yml)** on push/PR: `cargo fmt --check`,
      `cargo clippy -- -D warnings`, `cargo test --workspace`, and `npm ci && npm run build` for the
      web, with cargo + vendored-libgit2/openssl caching.
- [x] **Cross-platform matrix** (ubuntu/macos/windows) for `cargo test` — already in `ci.yml`.
- [x] **Image build + GHCR push on tags** — handled by [`release.yml`](../.github/workflows/release.yml)
      (see §5), so a separate `docker.yml` isn't needed.
- [ ] (Optional) Add `cargo deny check licenses` and the `#[ignore]`d testcontainers smoke
      (`make itest`) to CI.

## 5. Versioning & releases

- [x] **SemVer + [`CHANGELOG.md`](../CHANGELOG.md)** (Keep a Changelog format) in place.
- [x] **[`release.yml`](../.github/workflows/release.yml)** triggered on `v*` tags does it all:
      builds cross-platform `kanbanr` binaries (linux/macos-arm/macos-x86/windows) and attaches them
      to a GitHub Release, and pushes the **Docker image to GHCR** (`ghcr.io/startr-trade/kanbanr`).
- [ ] **Publish the CLI to crates.io** so `cargo install kanbanr` works. The workflow's `crates` job
      is wired but **guarded** — set repo variable `PUBLISH_CRATES=true` + the `CARGO_REGISTRY_TOKEN`
      secret (or switch to Trusted Publishing), then it publishes `kanbanr-core` then `kanbanr-cli`
      on tag. See the credentials section above.
- [ ] **Cut the first release:** `git tag v0.1.0 && git push origin v0.1.0`, then verify the Release
      assets + the GHCR image appear. Move the `[0.1.0]` section in the changelog from *Unreleased*
      to dated.
- [ ] **Publish the VS Code extension** ([`../editor/vscode/`](../editor/vscode/)) to the **VS Code
      Marketplace** (`vsce publish`, needs `VSCE_PAT`) and optionally **Open VSX** (`ovsx publish`)
      once you've smoke-tested it (F5 launch). Keep its `version` in step with releases.

## 6. Distributing the skill

The skill is the product surface for Claude users — make it trivial to install:

- [ ] Document manual install (symlink/copy `skill/kanbanr` → `~/.claude/skills/kanbanr`) — already
      in the guide; keep it front-and-center.
- [ ] Provide an install script / `make install-skill` that does the copy and verifies the CLI is
      on `PATH`.
- [ ] Investigate publishing via the **Claude plugin/skill marketplace** (the lowest-friction path
      for users) and link it from the README once available.
- [ ] If the MCP direction is taken (ADR-0006 on the board), document the MCP server install
      alongside the skill.

### Distribution as a Claude Code plugin (FEAT-023)

The lowest-friction install path: package the skill **and** the enforcement hooks together as a
**Claude Code plugin**, so users get both in one step instead of copying the skill and hand-editing
their `settings.json`. The plugin files live at the repo root:

- `.claude-plugin/plugin.json` — the plugin **manifest**. It references the existing skill via
  `"skills": ["${CLAUDE_PLUGIN_ROOT}/skill"]` (Claude Code discovers `skill/kanbanr/SKILL.md`) and
  registers the two hooks inline (`SessionStart` → `session-start`, `Stop` → `stop-check`) with
  `${CLAUDE_PLUGIN_ROOT}`-relative `command` paths — **never** hardcoded absolute paths, since the
  plugin is copied into a cache dir on install.
- `.claude-plugin/marketplace.json` — a one-plugin **marketplace catalog** (the plugin's `source`
  is `"./"`, the repo root). Relative-path sources resolve only when the marketplace is added via
  git, which is the intended distribution channel.

Users install with:

```shell
claude plugin marketplace add <github-owner>/kanbanr   # add this repo as a marketplace
claude plugin install kanbanr@kanbanr                  # install the plugin (skill + hooks)
```

(or the in-app `/plugin marketplace add` + `/plugin install` equivalents). Validate before
publishing with `claude plugin validate .`.

**Cross-platform hooks ship in two flavors:** Linux/macOS use the `.sh` scripts; Windows uses the
`.ps1` equivalents (`session-start.ps1`, `stop-check.ps1`) via a hook entry with
`"shell": "powershell"`. Both have the same best-effort, never-block semantics. See
[../skill/kanbanr/hooks/README.md](../skill/kanbanr/hooks/README.md).

**The plugin carries only the integration, not the program.** The `kanbanr` **binary** still ships
separately (crates.io via `cargo install kanbanr`, and/or GitHub Releases / GHCR per §5) and must be
on `PATH`; the hooks are best-effort and stay silent if it isn't installed. Keep the manifest's
`version` in step with releases (or omit it to let the git SHA version the plugin).

- [ ] Reserve the marketplace/plugin name and confirm it isn't an Anthropic-reserved name.
- [ ] Run `claude plugin validate .` in CI; verify install + the two hooks fire on a clean machine.
- [ ] Link the `claude plugin install` one-liner from the README once the repo is public.

## 7. Pre-announcement checklist

- [ ] Fresh clone → follow the README quickstart on a clean machine → you reach a populated board
      with **no undocumented step**. (Ideally on macOS/Windows too.)
- [ ] `make` / `cargo test` / web build all green in CI.
- [x] LICENSE(s), CONTRIBUTING, SECURITY, CODE_OF_CONDUCT, CHANGELOG, THIRD_PARTY present.
- [x] Owner slug (`startr-trade`) + copyright/email placeholders filled (see the top of this file).
- [ ] **Names reserved** (GitHub, crates.io); image + binaries published for the first tagged release.
- [ ] Dependabot is on, Discussions enabled (optional), `release.yml` secrets/variables set if publishing.
- [ ] Secrets sweep (§0) re-confirmed on the exact commit you'll make public.
- [ ] Screenshots (`make screenshots`) + a couple of example projects under `data/projects/` (non-sensitive).

## 8. After launch (keep it alive without burning out)

- [ ] Triage with labels; be explicit that it's a personal project (best-effort).
- [ ] Keep the board current so contributors know where to help — `kanbanr ready` is the answer
      to "what can I pick up?".
- [ ] Dependabot/`cargo update` + `npm audit` cadence; re-run `cargo deny`.
- [ ] Cut releases from `CHANGELOG.md`; don't let `main` drift far ahead of a tagged release.

---

### Minimal first-pass (the smallest credible public release)

Most of the scaffolding now exists in the repo. The **shortest path to a public v0.1.0** is just the
manual steps only you can do:

1. Create the `startr-trade/kanbanr` repository on GitHub (the owner slug is already filled in throughout).
2. Run the **secrets sweep** (§0); remove `data/security.yaml`; decide what `data/` to publish.
3. `git init` (if needed), commit, push, make the repo **public**.
4. Reserve the crate name + set up crates.io publishing (token **or** Trusted Publishing).
5. `git tag v0.1.0 && git push origin v0.1.0` → the release workflow builds binaries + the GHCR image.
6. Add screenshots to the README (`make screenshots`), then announce.

Everything else (Open VSX, npm reservation, MCP, badges polish) can follow.
