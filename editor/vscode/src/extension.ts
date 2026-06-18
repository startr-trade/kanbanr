// kanbanr — VS Code extension
//
// A thin viewer + command layer over the local kanbanr data folder. kanbanr is
// ONE local binary: `kanbanr serve` runs a read-only web monitor over the data
// folder (http://localhost:8080), while CLI subcommands perform local,
// git-backed writes (`board`, `activity`, `feature add`, `move`, ...).
//
// This extension does not embed any project logic of its own. It (a) opens the
// live monitor in a Webview (or the external browser) and (b) shells out to the
// `kanbanr` binary for the handful of write/read commands exposed in the
// command palette. The webview is intentionally a placeholder for a future
// native board UI — today it simply frames the running monitor.

import * as vscode from "vscode";
import { execFile } from "child_process";

/** Host/port the read-only monitor listens on (kanbanr serve default). */
const MONITOR_HOST = "127.0.0.1";
const MONITOR_PORT = 8080;
const MONITOR_URL = `http://localhost:${MONITOR_PORT}`;

/** Name of the binary we shell out to; expected to be on PATH. */
const KANBANR_BIN = "kanbanr";

/** Shared output channel for all CLI invocations. Created lazily on activate. */
let output: vscode.OutputChannel;

/**
 * Resolve the directory the CLI should run in. We prefer the first workspace
 * folder (the project root), falling back to undefined so child_process uses
 * the process cwd. The kanbanr CLI resolves its data folder relative to cwd
 * (or honours `--project`), so running from the workspace root is the
 * sensible default.
 */
function workspaceCwd(): string | undefined {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

/** Shape of a resolved CLI run. */
interface RunResult {
  stdout: string;
  stderr: string;
}

/**
 * Run the `kanbanr` binary with the given arguments via execFile (no shell, so
 * arguments are passed safely without quoting concerns). Streams stdout/stderr
 * to the output channel and rejects on failure. Callers handle the rejection
 * for user-facing messaging.
 */
function runKanbanr(args: string[]): Promise<RunResult> {
  return new Promise<RunResult>((resolve, reject) => {
    output.appendLine(`$ ${KANBANR_BIN} ${args.join(" ")}`);
    execFile(
      KANBANR_BIN,
      args,
      { cwd: workspaceCwd(), maxBuffer: 10 * 1024 * 1024 },
      (error, stdout, stderr) => {
        if (stdout) {
          output.append(stdout);
        }
        if (stderr) {
          output.append(stderr);
        }
        if (error) {
          reject(error);
          return;
        }
        resolve({ stdout, stderr });
      }
    );
  });
}

/**
 * Translate a child_process error into a friendly message. The common case is
 * ENOENT — the binary is not installed / not on PATH — for which we point the
 * user at `cargo install kanbanr`.
 */
function describeRunError(error: unknown): string {
  const err = error as NodeJS.ErrnoException;
  if (err && err.code === "ENOENT") {
    return `The 'kanbanr' binary was not found on your PATH. Install it with 'cargo install kanbanr', then reload the window.`;
  }
  return `kanbanr command failed: ${err?.message ?? String(error)}`;
}

/**
 * Probe whether the monitor is reachable. We open a TCP connection rather than
 * an HTTP request to avoid pulling in extra dependencies and to keep it fast.
 */
function isMonitorReachable(timeoutMs = 1000): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    // Lazy require so the module is only loaded when needed.
    const net = require("net") as typeof import("net");
    const socket = new net.Socket();
    let settled = false;

    const done = (reachable: boolean) => {
      if (settled) {
        return;
      }
      settled = true;
      socket.destroy();
      resolve(reachable);
    };

    socket.setTimeout(timeoutMs);
    socket.once("connect", () => done(true));
    socket.once("timeout", () => done(false));
    socket.once("error", () => done(false));
    socket.connect(MONITOR_PORT, MONITOR_HOST);
  });
}

/**
 * Start `kanbanr serve` in a dedicated integrated terminal. Reuses an existing
 * "kanbanr serve" terminal if one is already open so we don't spawn duplicates.
 */
function startServeTerminal(): vscode.Terminal {
  const existing = vscode.window.terminals.find((t) => t.name === "kanbanr serve");
  const terminal = existing ?? vscode.window.createTerminal({ name: "kanbanr serve", cwd: workspaceCwd() });
  terminal.show(true);
  terminal.sendText(`${KANBANR_BIN} serve`);
  return terminal;
}

/** Build the HTML that frames the live monitor inside a webview via <iframe>. */
function monitorWebviewHtml(): string {
  return `<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>kanbanr</title>
    <style>
      html, body { margin: 0; padding: 0; height: 100%; }
      iframe { border: 0; width: 100%; height: 100vh; display: block; }
    </style>
  </head>
  <body>
    <iframe src="${MONITOR_URL}" title="kanbanr live monitor"></iframe>
  </body>
</html>`;
}

/**
 * Command: kanbanr.openBoard
 *
 * Ensure the monitor is reachable at localhost:8080. If it is not, offer to
 * start `kanbanr serve`. Then show the monitor inside a WebviewPanel; if the
 * user prefers (or the webview iframe is blocked), fall back to opening the URL
 * externally.
 */
async function openBoard(): Promise<void> {
  let reachable = await isMonitorReachable();

  if (!reachable) {
    const choice = await vscode.window.showInformationMessage(
      `The kanbanr monitor is not running at ${MONITOR_URL}.`,
      "Start Server",
      "Cancel"
    );
    if (choice !== "Start Server") {
      return;
    }
    startServeTerminal();

    // Give the server a moment to come up, polling a few times.
    reachable = await waitForMonitor();
    if (!reachable) {
      vscode.window.showWarningMessage(
        `Started 'kanbanr serve' but the monitor isn't reachable yet. Try 'kanbanr: Open Board' again in a moment.`
      );
      return;
    }
  }

  const panel = vscode.window.createWebviewPanel(
    "kanbanrBoard",
    "kanbanr Board",
    vscode.ViewColumn.Active,
    { enableScripts: true, retainContextWhenHidden: true }
  );
  panel.webview.html = monitorWebviewHtml();

  // Offer an external-browser escape hatch in case the iframe is blocked by
  // the host's content policy.
  const openExternal = await vscode.window.showInformationMessage(
    "Showing the kanbanr monitor in a webview. Open in your browser instead?",
    "Open in Browser"
  );
  if (openExternal === "Open in Browser") {
    await vscode.env.openExternal(vscode.Uri.parse(MONITOR_URL));
  }
}

/** Poll the monitor for readiness for a short window after starting serve. */
async function waitForMonitor(attempts = 10, intervalMs = 500): Promise<boolean> {
  for (let i = 0; i < attempts; i++) {
    if (await isMonitorReachable(750)) {
      return true;
    }
    await new Promise((r) => setTimeout(r, intervalMs));
  }
  return false;
}

/**
 * Command: kanbanr.serve — start the monitor in an integrated terminal.
 */
function serve(): void {
  startServeTerminal();
  vscode.window.showInformationMessage(
    `Starting 'kanbanr serve'. The monitor will be available at ${MONITOR_URL}.`
  );
}

/**
 * Command: kanbanr.board — run `kanbanr board` and surface the snapshot in the
 * output channel.
 */
async function board(): Promise<void> {
  output.show(true);
  try {
    await runKanbanr(["board"]);
  } catch (error) {
    vscode.window.showErrorMessage(describeRunError(error));
  }
}

/**
 * Command: kanbanr.addFeature — prompt for the required title + milestone (and
 * optional kind/priority) and run `kanbanr feature add`.
 */
async function addFeature(): Promise<void> {
  const title = await vscode.window.showInputBox({
    title: "kanbanr: Add Feature",
    prompt: "Feature title (required)",
    ignoreFocusOut: true,
    validateInput: (value) => (value.trim().length === 0 ? "Title is required" : undefined),
  });
  if (title === undefined) {
    return; // user cancelled
  }

  const milestone = await vscode.window.showInputBox({
    title: "kanbanr: Add Feature",
    prompt: "Milestone (required)",
    ignoreFocusOut: true,
    validateInput: (value) => (value.trim().length === 0 ? "Milestone is required" : undefined),
  });
  if (milestone === undefined) {
    return;
  }

  // Optional fields — empty input means "omit this flag".
  const kind = await vscode.window.showInputBox({
    title: "kanbanr: Add Feature",
    prompt: "Kind (optional — leave blank to skip)",
    ignoreFocusOut: true,
  });
  if (kind === undefined) {
    return;
  }

  const priority = await vscode.window.showInputBox({
    title: "kanbanr: Add Feature",
    prompt: "Priority (optional — leave blank to skip)",
    ignoreFocusOut: true,
  });
  if (priority === undefined) {
    return;
  }

  const args = ["feature", "add", "--title", title.trim(), "--milestone", milestone.trim()];
  if (kind.trim().length > 0) {
    args.push("--kind", kind.trim());
  }
  if (priority.trim().length > 0) {
    args.push("--priority", priority.trim());
  }

  output.show(true);
  try {
    await runKanbanr(args);
    vscode.window.showInformationMessage(`Added feature: ${title.trim()}`);
  } catch (error) {
    vscode.window.showErrorMessage(describeRunError(error));
  }
}

/**
 * Command: kanbanr.move — prompt for a feature code + target status and run
 * `kanbanr move <CODE> <STATUS>`.
 */
async function move(): Promise<void> {
  const code = await vscode.window.showInputBox({
    title: "kanbanr: Move Feature",
    prompt: "Feature code (e.g. FEAT-014)",
    ignoreFocusOut: true,
    validateInput: (value) => (value.trim().length === 0 ? "Feature code is required" : undefined),
  });
  if (code === undefined) {
    return;
  }

  const status = await vscode.window.showInputBox({
    title: "kanbanr: Move Feature",
    prompt: "Target status (e.g. In progress, Done)",
    ignoreFocusOut: true,
    validateInput: (value) => (value.trim().length === 0 ? "Status is required" : undefined),
  });
  if (status === undefined) {
    return;
  }

  output.show(true);
  try {
    await runKanbanr(["move", code.trim(), status.trim()]);
    vscode.window.showInformationMessage(`Moved ${code.trim()} → ${status.trim()}`);
  } catch (error) {
    vscode.window.showErrorMessage(describeRunError(error));
  }
}

/**
 * Extension entry point. Registers every command and the shared output channel
 * with the extension context so they are disposed on deactivation.
 */
export function activate(context: vscode.ExtensionContext): void {
  output = vscode.window.createOutputChannel("kanbanr");
  context.subscriptions.push(output);

  context.subscriptions.push(
    vscode.commands.registerCommand("kanbanr.openBoard", () => openBoard()),
    vscode.commands.registerCommand("kanbanr.serve", () => serve()),
    vscode.commands.registerCommand("kanbanr.board", () => board()),
    vscode.commands.registerCommand("kanbanr.addFeature", () => addFeature()),
    vscode.commands.registerCommand("kanbanr.move", () => move())
  );
}

/** No-op deactivate — disposables are handled via context.subscriptions. */
export function deactivate(): void {
  // Intentionally empty.
}
