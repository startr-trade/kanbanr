# Installation

kanbanr ships as **one binary**. The CLI, the read-only web monitor and its assets are the same
executable — there is no separate UI to build and nothing to point `serve` at.

| | What it is | How you get it |
|---|---|---|
| **`kanbanr`** | the CLI (single writer) *and* the web monitor (`kanbanr serve`) | one-line install, below |
| **container image** | the same binary in serve mode, for running the monitor somewhere else | `docker pull`, optional |

Neither needs a Rust toolchain. You only need one to build from source, which is the last section
on this page.

## Install

```bash
curl -fsSL https://github.com/startr-trade/kanbanr/releases/latest/download/install.sh | sh
```

On Windows (PowerShell):

```powershell
irm https://github.com/startr-trade/kanbanr/releases/latest/download/install.ps1 | iex
```

The installer downloads the release build for your platform, **verifies its SHA-256 against the
release's own `SHA256SUMS`**, and installs it — into `/usr/local/bin` when that is writable,
otherwise `~/.local/bin`. It edits no shell profile and starts no daemon.

The one-liner is served from the **release** rather than from `raw.githubusercontent.com` on
purpose: the raw host is a CDN of the default branch, so it can hand out an installer that does not
match the release it is installing, and it rate-limits harder. From the release download host the
script and the binaries it fetches are one artifact set, versioned together.

Useful knobs:

```bash
# Pin a version instead of taking the newest release
curl -fsSL …/install.sh | sh -s -- --version v0.1.0

# Install somewhere specific
curl -fsSL …/install.sh | KANBANR_INSTALL_DIR=~/bin sh

# Lift the API rate limit when retrying (60 requests/hour anonymous -> 5000 with a token)
GH_TOKEN=$(gh auth token) curl -fsSL …/install.sh | sh
```

A token is sent **only** to `api.github.com`, which is the endpoint that rate-limits. The release
download host needs no credential and is never given one — an installer that forwards your token to
every host it touches is a worse trade than a slower retry.

### If the install fails

`raw.githubusercontent.com` **rate-limits** and answers `429 Too Many Requests` under load; so does
the GitHub API, at 60 requests an hour per IP without a token. Three ways around it, in the order
worth trying:

```bash
# 1. pin the version, which skips the release lookup entirely — the only path that needs no API call
curl -fsSL https://github.com/startr-trade/kanbanr/releases/download/v0.1.0/install.sh \
  | sh -s -- --version v0.1.0

# 2. a token, which moves the API from 60 to 5000 requests an hour
GH_TOKEN=$(gh auth token) curl -fsSL …/install.sh | sh

# 3. no installer at all — the assets are plain files
gh release download v0.1.0 -R startr-trade/kanbanr \
  -p 'kanbanr-*-x86_64-unknown-linux-gnu.tar.gz' -p SHA256SUMS
sha256sum --ignore-missing -c SHA256SUMS
tar xzf kanbanr-*-x86_64-unknown-linux-gnu.tar.gz -C ~/.local/bin
```

The installer retries transient failures and times out rather than hanging, and it resolves the
release from three different endpoints (`/releases/latest`, the release list, then git tags),
because each of them has been observed failing while a release was perfectly installable. When all
three come up empty it tells you to pin `--version`, which is the one path that needs no lookup.

Prefer to see what you are running before you run it? Download `install.sh`, read it — it is about
240 lines of POSIX shell, and CI shellchecks it — then execute it. Or skip the script and
take the archive from the [releases page](https://github.com/startr-trade/kanbanr/releases).

Verify:

```bash
kanbanr --version
kanbanr serve          # the monitor, from the binary — no --ui-dir, no Node
```

### Published targets

**linux x86_64**, **linux aarch64**, **macOS arm64**, **macOS x86_64**, **windows x86_64**.

The Linux builds are **glibc**, not static musl, because kanbanr links libgit2. They are built on
the oldest runner we support (Ubuntu 22.04, glibc 2.35), so they run on Debian bookworm and
anything newer. On an older distribution than that, build from source or use the container image —
a glibc mismatch shows up as `libc.so.6: version 'GLIBC_2.xx' not found` at startup, not as a
subtle failure.

## Install the skill

The program is half of kanbanr; the other half is the **skill** that teaches Claude Code to use it.
**The program carries its skill**, and the installer above puts it in place when it finds Claude
Code (the `claude` command) — so usually there is nothing to do here. The skill always matches the
program: `kanbanr self-update` updates both.

```bash
kanbanr skill status       # is it installed, and does it match this program?
kanbanr skill install      # write it to ~/.claude/skills/kanbanr (or $CLAUDE_CONFIG_DIR/skills)
kanbanr skill uninstall    # remove the copy kanbanr installed
```

A skill folder kanbanr did not write — a link to a clone, a plugin's copy — is left alone. The
other ways below are for people who want them; pick **one** route, or Claude Code loads two copies
of the skill.

**As a Claude Code plugin** — the skill plus its session-start, stop and session-summary hooks, from
the repository's `main` rather than your release:

```bash
claude plugin marketplace add startr-trade/kanbanr
claude plugin install kanbanr@kanbanr
```

**From a clone** — for working on the skill itself: a link into Claude Code's personal skills
folder, updated by `git pull`:

```bash
git clone https://github.com/startr-trade/kanbanr ~/.local/share/kanbanr
make -C ~/.local/share/kanbanr install-skill      # links ~/.claude/skills/kanbanr
```

With the clone, the hooks come from `kanbanr hooks install` in each project, which `kanbanr init`
and the setup interview run for you. Either way, then open Claude Code in a project and say **"set up
kanbanr for this project"**.

## The VS Code extension

An optional extension opens the monitor inside your editor and runs the common board commands
through the CLI. Every release carries it as `kanbanr-vscode-<version>.vsix`, at the program's
version and listed in the release's `SHA256SUMS`.

**With the installer**: add `--vscode`, and it installs the extension into every editor it finds
(`code`, `codium`, `cursor`, `windsurf`), or `--vscode=codium` for one. Without the option it
installs nothing into any editor.

```bash
curl -fsSL https://github.com/startr-trade/kanbanr/releases/latest/download/install.sh | sh -s -- --vscode
```
```powershell
$env:KANBANR_VSCODE = 1; irm https://github.com/startr-trade/kanbanr/releases/latest/download/install.ps1 | iex
```

**From the program**, at any time: `kanbanr editor install` (or `--editor codium`) downloads the
`.vsix` of the version you are running, checks it against `SHA256SUMS`, and installs it.
`kanbanr self-update` then keeps it in step in every editor that has it.

**From Open VSX**: VSCodium, Cursor, Windsurf and other editors that use
[Open VSX](https://open-vsx.org/extension/kanbanr/kanbanr) can install it from their Extensions view
(search "kanbanr"), or `codium --install-extension kanbanr.kanbanr`, and update it from there.
VS Code itself does not read Open VSX.

**By hand**, from a release:

```bash
v=0.1.4
curl -fsSLO https://github.com/startr-trade/kanbanr/releases/download/v$v/kanbanr-vscode-$v.vsix
code --install-extension kanbanr-vscode-$v.vsix
```

## Run the monitor somewhere else

```bash
docker pull ghcr.io/startr-trade/kanbanr:latest
docker run --rm -p 8080:8080 -v "$(kanbanr where)":/data ghcr.io/startr-trade/kanbanr:latest
```

The image is the same binary in serve mode, with the board mounted at `/data`. It is read-only and
unauthenticated: expose it beyond localhost only behind a reverse proxy you control.

## Staying current

```bash
kanbanr self-update --check          # is this binary current? changes nothing
kanbanr self-update                  # replace it with what the release publishes
kanbanr self-update --version v0.1.0 # pin, or roll back
```

Updates are **never automatic** — nothing runs on a timer or as a side effect of another command.
What an update may and may not change — the board format, commands, `--json` — is set out in
[Stability](../project/stability.md).

### What "current" means here

`--check` reports **two different** reasons a binary can be out of date:

1. **A newer version** — the latest release tag differs from yours.
2. **The same version, rebuilt** — the tag matches, but the binary the release publishes is not the
   one you are running.

The second is the one most tools miss. An asset re-uploaded under the same tag — an interim fix, a
corrected packaging step, a re-run workflow — changes nothing a version comparison can see. So the
check is made on the **SHA-256 of the binary**, against a per-target checksum the release publishes
beside the archives.

The commit hash cannot do that job, which is why both exist:

| | Answers | Decides? |
|---|---|---|
| **SHA-256 of the binary** | *is the binary I am running the one this release publishes?* | **yes** — it identifies the artifact, so it catches a rebuild, a re-upload, a corrupted install, or a locally built binary that merely shares a version |
| **commit hash** (in `--version`, and on the release page) | *what source was it built from?* | no — two different binaries routinely share one commit (a re-run workflow, a toolchain bump). But when the checksums differ it is the only thing that says **why** |

So a rebuild is reported as *"same version, same commit, different binary"*, and a tag moved onto
other source as *"now points at a different commit"* — which is unusual enough to be worth saying
out loud rather than folding into "an update is available".

```console
$ kanbanr --version
kanbanr 0.1.0 (a1b2c3d4e5f6, built 2026-09-28)
```

That commit is the one the GitHub release page shows for the tag, so an installed binary can be
matched against what is published without running anything. CI fails a release whose binaries report
`unknown` or `dirty`.

A release published before the per-binary checksum existed cannot answer the question, and
`--check` says exactly that rather than claiming you are current.

### Everything is fetched over HTTPS

Both the updater and the installers refuse a non-`https` URL before sending, and they follow
redirects **themselves** rather than letting the HTTP client do it, checking each hop's scheme —
because a release download *is* a redirect (`github.com` answers a 302 to
`objects.githubusercontent.com`), and both `ureq` and `curl -L` will happily follow one that
downgrades to plaintext.

A checksum does not make plaintext acceptable here: `SHA256SUMS` arrives over the same channel as
the archive it vouches for, so anyone able to rewrite one can rewrite the other. Verification only
holds when the thing doing the vouching arrived over a channel that was authenticated.

The GitHub token, when you supply one, goes only to `api.github.com` and is **never carried across a
redirect** — not even to another GitHub host.

### How an update is installed

The archive is verified against the release's `SHA256SUMS`, the binary extracted from it is verified
against its own published checksum, and only then is it moved into place by an **atomic rename** —
so an interrupted update cannot leave a half-written executable on your `PATH`. A checksum mismatch
aborts, and there is no flag to get past it: anyone who genuinely wants an unverified binary can
download it by hand and see what they are doing.

Re-running the installer works too, and is the path on Windows, where a running `.exe` cannot be
replaced from inside itself.

Your board is untouched by any of this: it is a separate git repository beside your code, and the
binary holds no state.

## Build from source

You need a stable [Rust toolchain](https://rustup.rs) **and Node**. Node is not needed to *use*
kanbanr — the released binary has the monitor baked in — but it is needed to *build* one, because
that is the step that produces the assets to bake (see `ADR-0009`).

```bash
git clone https://github.com/startr-trade/kanbanr.git
cd kanbanr

make test          # the whole suite, no Docker
make install       # builds the SPA, installs the binary, links the Claude skill
```

`cargo install --path api/crates/kanbanr-cli` on its own works too, but without a built `web/dist`
present it embeds no monitor — `kanbanr serve` then says so at startup and tells you what to do.
That is also why **kanbanr is not published to crates.io**: a published crate cannot carry the
built assets without committing generated files to the repository, so the archive and the installer
are the supported way to get a complete binary. (The one crate that is published is
`ears-classifier`, the standalone EARS library kanbanr uses.)

## Next

**[Everyday use](../using/everyday.md)** — the board, the method, and what the CLI can do.
