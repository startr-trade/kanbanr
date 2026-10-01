#!/usr/bin/env python3
"""Session summaries, saved to the kanbanr board (FEAT-099).

    summarise_session.py compact    <session_id> <trigger> <transcript_path>   # summary text on stdin
    summarise_session.py transcript <transcript_path> <why>
    summarise_session.py sweep      <transcripts_dir> <current_session_id>
    summarise_session.py backfill   <transcripts_dir>                          # by hand

Summaries are files beside the board, `<board>/.sessions/<project>/<YYYY-MM-DD>-<HHMMSS>-<sid8>.md`:
dated and timed so several sessions in a day never collide, and carrying the session id prefix so
each maps back to its transcript. They are never committed (every board ignores `.sessions/`,
FEAT-135): a summary is a condensed conversation, and a board is what a team shares. Nothing is
written to the code repository, and nothing is imported into CLAUDE.md — the board is already what
a new session recovers from, so a summary is a record to consult, not context to load.

The transcript is never piped in whole. This project's was 49 MB, about 13M tokens, and a summary is
wanted exactly when the context is full. What is sent is the conversation only: the latest summary
Claude Code wrote at compaction, plus the user and assistant text after it. Tool output, thinking and
attachments are dropped.
"""
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

PROJECT = Path(os.environ.get("CLAUDE_PROJECT_DIR") or os.getcwd())
# The prompt ships beside this script (FEAT-101). A project that keeps its own `/create-summary`
# command gets that instead, so the summaries it saves match the ones its people ask for by hand.
PROMPT_FILE = next(
    (p for p in (PROJECT / ".claude" / "commands" / "create-summary.md",
                 Path(__file__).resolve().parent / "session-summary.prompt.md") if p.exists()),
    None,
)
MAX_CHARS = 300_000        # ~75k tokens: far inside a fresh window, with room for the prompt
MAX_PER_SWEEP = 3          # bound the cost of the first start after a long gap
MIN_NEW_MESSAGES = 3       # a resumed session is re-summarised only once it has really moved on
SWEEP_DAYS = 14


def state_dir(transcripts: Path) -> Path:
    d = transcripts / ".session-summaries"
    d.mkdir(exist_ok=True)
    return d


def log(transcripts: Path, msg: str) -> None:
    with open(state_dir(transcripts) / "summaries.log", "a") as f:
        f.write(f"{datetime.datetime.now().isoformat(timespec='seconds')}  {msg}\n")


def saved_compactions(transcripts: Path, sid: str) -> Path:
    return state_dir(transcripts) / f"{sid}.compactions"


def compaction_key(summary: str) -> str:
    import hashlib
    return hashlib.sha256(summary.strip().encode()).hexdigest()[:16]


def already_saved(transcripts: Path, sid: str, summary: str) -> bool:
    f = saved_compactions(transcripts, sid)
    return f.exists() and compaction_key(summary) in f.read_text().split()


def mark_saved(transcripts: Path, sid: str, summary: str) -> None:
    with open(saved_compactions(transcripts, sid), "a") as f:
        f.write(compaction_key(summary) + "\n")


# Things the user has said must never reach the board (FEAT-124). The list lives OUTSIDE the
# repository and the board — this code only knows where to look — one term per line, matched
# case-insensitively; blank lines and `#` comments are ignored.
EXCLUDE_FILE = Path(os.environ.get("KANBANR_SUMMARY_EXCLUDE")
                    or Path.home() / ".claude" / "kanbanr-summary-exclude.txt")


def excluded_terms() -> list:
    """The private terms, or [] when there is no list. A list that exists but cannot be read raises:
    silently summarising without it would be exactly the leak it exists to prevent."""
    if not EXCLUDE_FILE.exists():
        return []
    lines = EXCLUDE_FILE.read_text().splitlines()
    return [t.strip().lower() for t in lines if t.strip() and not t.strip().startswith("#")]


def mentions_excluded(text: str, terms: list) -> bool:
    """Whole-word match, so a short term does not catch every longer word that happens to hold it."""
    lower = text.lower()
    return any(re.search(r"(?<![a-z0-9])" + re.escape(t) + r"(?![a-z0-9])", lower) for t in terms)


def redact(text: str, terms: list) -> str:
    """Remove every line that mentions an excluded term. Lines, not words: a sentence with the word
    cut out still says what it was about."""
    if not terms:
        return text
    return "\n".join(l for l in text.splitlines() if not mentions_excluded(l, terms))


def kanbanr() -> str:
    return shutil.which("kanbanr") or str(Path.home() / ".cargo" / "bin" / "kanbanr")


def save_to_board(name: str, body: str) -> str:
    """Write `body` to `<board>/.sessions/<project>/<name>.md`, beside the board and outside its
    history (FEAT-135). The board and project are kanbanr's answer for the project directory — its
    .kanbanr marker — so this must run there; nothing is committed."""
    where = json.loads(subprocess.run(
        [kanbanr(), "where", "--json"],
        cwd=PROJECT, check=True, capture_output=True, text=True, timeout=60,
    ).stdout)
    folder = Path(where["data_dir"]) / ".sessions" / where["project"]
    folder.mkdir(parents=True, exist_ok=True)
    path = folder / f"{name}.md"
    # The one door out, so the one place the exclusion list is enforced on the way.
    body = redact(body, excluded_terms()) + "\n"
    tmp = path.with_suffix(".md.tmp")
    tmp.write_text(body)
    tmp.replace(path)
    return str(path)


def text_of(entry: dict) -> str:
    content = (entry.get("message") or {}).get("content")
    if isinstance(content, str):
        parts = [content]
    elif isinstance(content, list):
        parts = [b.get("text", "") for b in content if isinstance(b, dict) and b.get("type") == "text"]
    else:
        parts = []
    text = "\n".join(parts)
    return re.sub(r"<system-reminder>.*?</system-reminder>", "", text, flags=re.S).strip()


def read(transcript: Path) -> list:
    entries = []
    with open(transcript) as f:
        for line in f:
            try:
                entries.append(json.loads(line))
            except ValueError:
                continue  # a line being written as we read
    return entries


def conversation(entries: list):
    """(earlier-summary, [(role, text)] after it). Only what was said, not what tools returned.

    A message that mentions an excluded term is dropped whole, before the model sees anything
    (FEAT-124): what the summariser is never told, it cannot repeat."""
    terms = excluded_terms()
    last = max((i for i, e in enumerate(entries) if e.get("isCompactSummary")), default=-1)
    earlier = redact(text_of(entries[last]), terms) if last >= 0 else ""
    messages = []
    for e in entries[last + 1:]:
        if e.get("type") not in ("user", "assistant") or e.get("isMeta") or e.get("isCompactSummary"):
            continue
        t = text_of(e)
        if t and not mentions_excluded(t, terms):
            messages.append((e["type"], t))
    return earlier, messages


def condense(earlier: str, messages: list) -> str:
    out = []
    if earlier:
        out.append("## Summary of the earlier part of the session (written at compaction)\n\n" + earlier)
    out.append("## The conversation since\n")
    out += [f"**{role.upper()}:** {t}" for role, t in messages]
    text = "\n\n".join(out)
    if len(text) > MAX_CHARS:
        # Keep the earlier summary and the most recent conversation: the end is what a returning
        # reader most needs, and the summary already covers the start.
        head = out[0] if earlier else ""
        text = head + "\n\n[... the middle of the conversation was omitted for length ...]\n\n" + text[-(MAX_CHARS - len(head)):]
    return text


def summarise_with_claude(condensed: str) -> str:
    prompt = PROMPT_FILE.read_text() if PROMPT_FILE else "Summarise this Claude Code session."
    prompt = re.sub(r"\A---\n.*?\n---\n", "", prompt, flags=re.S).strip()
    prompt += (
        "\n\nThe session is on standard input, condensed: a summary of its earlier part written when "
        "the context was compacted, then the user and assistant messages since. Tool output was "
        "removed, so name files and commands only where the conversation itself does. Do not add "
        "a title or a date — the document already has both; begin with the Objective section."
    )
    env = dict(os.environ, KANBANR_SESSION_SUMMARY="1")
    # One fixed, neutral working directory, user settings only, no persisted session: the summariser
    # must not fire this project's hooks (which would summarise it in turn) or leave a transcript.
    # Fixed rather than a fresh temp dir, because Claude Code creates a (memory-only) project folder
    # per working directory, and a fresh one per run left one empty folder behind every time.
    neutral = Path.home() / ".claude" / "session-summariser"
    neutral.mkdir(parents=True, exist_ok=True)
    done = subprocess.run(
        ["claude", "-p", "--model", "sonnet", "--no-session-persistence",
         "--setting-sources", "user", prompt],
        input=condensed, cwd=neutral, env=env, capture_output=True, text=True, timeout=900,
    )
    if done.returncode != 0 or not done.stdout.strip():
        raise RuntimeError(f"claude -p failed ({done.returncode}): {done.stderr.strip()[:500]}")
    # The model sometimes returns the markdown inside a ```markdown fence, which the board would then
    # show as a code block. Unwrap a fence that encloses the whole answer; leave inner fences alone.
    out = done.stdout.strip()
    fenced = re.fullmatch(r"```(?:markdown|md)?\s*\n(.*)\n```", out, flags=re.S)
    out = fenced.group(1).strip() if fenced else out
    # One title per doc, and it is the header's: the model does not know the session's dates and
    # tends to title with today's, which contradicts the header (it wrote "2026-09-29" under a
    # header dated 11 June). Drop a leading H1 if it adds one anyway.
    return re.sub(r"\A#\s[^\n]*\n+", "", out).strip()


def header(title: str, sid: str, source: str, transcript: str, when: datetime.datetime,
           extra: str = "") -> str:
    return (
        f"# {title} — {when.strftime('%Y-%m-%d %H:%M:%S')}\n\n"
        f"- **Session:** `{sid}`\n{extra}- **Source:** {source}\n- **Transcript:** `{transcript}`\n\n---\n\n"
    )


def stamp(entry: dict):
    ts = entry.get("timestamp")
    try:
        return datetime.datetime.fromisoformat(ts.replace("Z", "+00:00")).astimezone() if ts else None
    except ValueError:
        return None


def span(entries: list, fmt: str = "%Y-%m-%d %H:%M"):
    """(first activity, last activity, last compaction) — the dates a reader needs to know what a
    summary of a long, repeatedly compacted session actually covers."""
    times = [t for t in (stamp(e) for e in entries) if t]
    compactions = [t for t in (stamp(e) for e in entries if e.get("isCompactSummary")) if t]
    first, last = (times[0], times[-1]) if times else (None, None)
    return first, last, (compactions[-1] if compactions else None)


def started_at(entries: list, fallback: float) -> datetime.datetime:
    for e in entries:
        ts = e.get("timestamp")
        if ts:
            try:
                return datetime.datetime.fromisoformat(ts.replace("Z", "+00:00")).astimezone()
            except ValueError:
                pass
    return datetime.datetime.fromtimestamp(fallback).astimezone()


def backfill_compactions(entries: list, transcript: Path) -> int:
    transcripts, sid = transcript.parent, transcript.stem
    saved = 0
    for e in entries:
        if not e.get("isCompactSummary"):
            continue
        summary = text_of(e)
        if not summary or already_saved(transcripts, sid, summary):
            continue
        when = started_at([e], transcript.stat().st_mtime)
        name = f"{when.strftime('%Y-%m-%d-%H%M%S')}-{sid[:8]}-compact"
        body = header("Compaction summary", sid, "compaction (recovered from the transcript)",
                      str(transcript), when) + summary + "\n"
        doc = save_to_board(name, body)
        mark_saved(transcripts, sid, summary)
        log(transcripts, f"{sid}: backfilled compaction -> {doc}")
        saved += 1
    return saved


def summarise_transcript(transcript: Path, why: str) -> None:
    transcripts, sid = transcript.parent, transcript.stem
    state = state_dir(transcripts)
    marker, lock = state / f"{sid}.json", state / f"{sid}.lock"
    try:
        lock.mkdir()
    except FileExistsError:
        if time.time() - lock.stat().st_mtime < 1800:
            return  # another summariser is on it
        lock.rmdir()
        lock.mkdir()
    try:
        entries = read(transcript)
        backfill_compactions(entries, transcript)
        earlier, messages = conversation(entries)
        seen = json.loads(marker.read_text()).get("messages", -1) if marker.exists() else -1
        count = len(messages) + (1 if earlier else 0)
        if not messages and not earlier:
            marker.write_text(json.dumps({"messages": 0, "doc": None}))
            return
        if seen >= 0 and count - seen < MIN_NEW_MESSAGES:
            return
        summary = summarise_with_claude(condense(earlier, messages))
        when = started_at(entries, transcript.stat().st_mtime)
        first, last, compacted = span(entries)
        fmt = "%Y-%m-%d %H:%M"
        extra = ""
        if first and last:
            extra += f"- **Session ran:** {first.strftime(fmt)} → {last.strftime(fmt)}\n"
        if compacted:
            n = sum(1 for e in entries if e.get("isCompactSummary"))
            extra += (f"- **This summary covers:** {compacted.strftime(fmt)} → {last.strftime(fmt)}, "
                      f"after the last of {n} compactions. The earlier parts are in this session's "
                      f"`-compact` docs.\n")
        else:
            extra += "- **This summary covers:** the whole session\n"
        name = f"{when.strftime('%Y-%m-%d-%H%M%S')}-{sid[:8]}"
        body = header("Session summary", sid, why, str(transcript), when, extra) + summary + "\n"
        doc = save_to_board(name, body)
        marker.write_text(json.dumps({"messages": count, "doc": doc}))
        log(transcripts, f"{sid}: {why} -> {doc}")
    except Exception as e:  # a hook must never break the session; the log says what happened
        log(transcripts, f"{sid}: {why} FAILED: {e}")
    finally:
        lock.rmdir()


def save_compact(sid: str, trigger: str, transcript: str) -> None:
    summary = sys.stdin.read().strip()
    transcripts = Path(transcript).parent
    if not summary:
        log(transcripts, f"{sid}: PostCompact carried no summary")
        return
    now = datetime.datetime.now().astimezone()
    name = f"{now.strftime('%Y-%m-%d-%H%M%S')}-{sid[:8]}-compact"
    body = header("Compaction summary", sid, f"compaction ({trigger})", transcript, now) + summary + "\n"
    if already_saved(transcripts, sid, summary):
        return
    try:
        log(transcripts, f"{sid}: compaction ({trigger}) -> {save_to_board(name, body)}")
        mark_saved(transcripts, sid, summary)
    except Exception as e:
        log(transcripts, f"{sid}: compaction FAILED: {e}")


def sweep(transcripts: Path, current: str) -> None:
    cutoff = time.time() - SWEEP_DAYS * 86400
    candidates = sorted(
        (p for p in transcripts.glob("*.jsonl") if p.stem != current and p.stat().st_mtime > cutoff),
        key=lambda p: p.stat().st_mtime, reverse=True,
    )
    done = 0
    for p in candidates:
        if done >= MAX_PER_SWEEP:
            break
        before = (state_dir(transcripts) / f"{p.stem}.json").read_text() if (state_dir(transcripts) / f"{p.stem}.json").exists() else None
        summarise_transcript(p, "recovered at the next session start")
        after = (state_dir(transcripts) / f"{p.stem}.json").read_text() if (state_dir(transcripts) / f"{p.stem}.json").exists() else None
        done += before != after


if __name__ == "__main__":
    if os.environ.get("KANBANR_SESSION_SUMMARY"):
        sys.exit(0)
    mode, args = sys.argv[1], sys.argv[2:]
    if mode == "compact":
        save_compact(*args)
    elif mode == "transcript":
        summarise_transcript(Path(args[0]), args[1])
    elif mode == "sweep":
        sweep(Path(args[0]), args[1])
    elif mode == "backfill":
        # Every transcript in a folder, the open session included: saves each compaction summary it
        # holds and summarises the conversation after the last. Safe to re-run.
        for t in sorted(Path(args[0]).glob("*.jsonl")):
            summarise_transcript(t, "backfilled by hand")
