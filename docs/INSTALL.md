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

## Run the monitor somewhere else

```bash
docker pull ghcr.io/startr-trade/kanbanr:latest
docker run --rm -p 8080:8080 -v "$(kanbanr where)":/data ghcr.io/startr-trade/kanbanr:latest
```

The image is the same binary in serve mode, with the board mounted at `/data`. It is read-only and
unauthenticated: expose it beyond localhost only behind a reverse proxy you control.

## Staying current

There is no `self-update` yet — re-run the installer, which replaces the binary in place:

```bash
curl -fsSL https://github.com/startr-trade/kanbanr/releases/latest/download/install.sh | sh
```

Your board is untouched by this: it is a separate git repository beside your code, and the binary
holds no state.

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
That is also why **kanbanr is not published to crates.io with the UI**: a published crate cannot
carry the built assets without committing generated files to the repository, so the archive and the
installer are the supported way to get a complete binary.

## Next

**[USER_GUIDE.md](USER_GUIDE.md)** — the board, the method, and what the CLI can do.
