# kanbanr — VS Code extension

A VS Code **viewer and thin command layer** for [kanbanr](https://kanbanr.startr.trade), the
local, git-backed project management system of record.

kanbanr is **one local binary**. `kanbanr serve` runs a read-only web monitor
over your data folder at <http://localhost:8080>, and the CLI performs local,
git-backed writes (`kanbanr board`, `kanbanr feature add`, `kanbanr move`, …).
The board viewer is pluggable — this extension is the VS Code viewer plus a thin
wrapper over the same single runtime. It holds no project state of its own; it
shells out to the `kanbanr` binary and frames the live monitor.

## What it does

This extension contributes the following commands (Command Palette → `kanbanr:`):

| Command                          | Action |
| -------------------------------- | ------ |
| **kanbanr: Open Board (Live Monitor)** | Ensures the monitor is reachable at `localhost:8080` (offers to start `kanbanr serve` if not), then frames it in a Webview `<iframe>`. Falls back to opening the URL in your browser. |
| **kanbanr: Start Server**        | Runs `kanbanr serve` in an integrated terminal. |
| **kanbanr: Show Board (Text Snapshot)** | Runs `kanbanr board` and prints the snapshot to the *kanbanr* output channel. |
| **kanbanr: Add Feature**         | Prompts for title + milestone (and optional kind/priority) and runs `kanbanr feature add`. |
| **kanbanr: Move Feature**        | Prompts for a feature code + target status and runs `kanbanr move <CODE> <STATUS>`. |

CLI commands run from the first workspace folder (the project root), so kanbanr
resolves its data folder relative to that directory. Output and errors are
surfaced via the *kanbanr* output channel and VS Code notifications.

## Requirements

- The **`kanbanr` binary must be on your `PATH`**. Install it with the release installer
  (kanbanr is not published to crates.io):

  ```sh
  # Linux / macOS
  curl -fsSL https://github.com/startr-trade/kanbanr/releases/latest/download/install.sh | sh
  # Windows (PowerShell)
  irm https://github.com/startr-trade/kanbanr/releases/latest/download/install.ps1 | iex
  ```

  If the binary is missing, the extension tells you exactly this when you run a
  command.

## Install

Each [kanbanr release](https://github.com/startr-trade/kanbanr/releases) carries the extension as
`kanbanr-vscode-<version>.vsix`, the same version as the program (and covered by the release's
`SHA256SUMS`):

```sh
gh release download -R startr-trade/kanbanr -p 'kanbanr-vscode-*.vsix'
code --install-extension kanbanr-vscode-*.vsix
```

Or, in VS Code: **Extensions → ⋯ → Install from VSIX…**. VSCodium, Cursor, Windsurf and other
editors that use [Open VSX](https://open-vsx.org) can install it from there once it is published
under the `kanbanr` namespace.

## Build & run (development)

This extension is authored in TypeScript and compiled with `tsc`. No bundler.

```sh
cd editor/vscode
npm ci             # the pinned toolchain: typescript, @vscode/vsce, ovsx
npm run compile    # tsc -p ./  →  emits out/extension.js
npm run package    # a .vsix, as the release builds it
```

Then open this folder in VS Code and press **F5** (Run Extension) to launch an
Extension Development Host with the extension loaded. Use `npm run watch` for an
incremental rebuild while developing.

## Roadmap

This is the scaffold for roadmap item **FEAT-014**. The Webview currently frames
the running `kanbanr serve` monitor; it is the intended future home of a native
board webview rendered directly from the local data folder, keeping the same
single-runtime model.
