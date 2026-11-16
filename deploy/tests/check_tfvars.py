"""Keep the hosted site's secrets and state out of git (FTR-030 R-8).

Run from the repository root:

    python3 deploy/tests/check_tfvars.py

It exits 1 with one line per finding, and 0 when there is none:

  (a) a tracked *.sensitive.auto.tfvars, *.tfstate, *.tfstate.* or .terraform/ path;
  (b) a tracked *.tfvars (not *.example) that assigns a variable marked sensitive;
  (c) a string in a *.sensitive.auto.tfvars.example that is not "" or a "<...>" placeholder;
  (d) a .gitignore missing *.sensitive.auto.tfvars, .terraform/, *.tfstate or *.tfstate.*.

Python standard library only.
"""

import fnmatch
import re
import subprocess
import sys
from pathlib import Path, PurePosixPath

GITIGNORE_PATTERNS = ("*.sensitive.auto.tfvars", ".terraform/", "*.tfstate", "*.tfstate.*")
SECRET_FILE = "*.sensitive.auto.tfvars"
EXAMPLE_FILE = "*.sensitive.auto.tfvars.example"
PLACEHOLDER = re.compile(r"^<[^<>]*>$")


def tracked_files(root="."):
    """Every path git tracks under root, '/'-separated and relative to it."""
    out = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z"],
        check=True,
        capture_output=True,
    ).stdout
    return [p for p in out.decode("utf-8", "surrogateescape").split("\0") if p]


def _tokens(text):
    """Yield (kind, value, depth, start) for HCL text: kind is 'string',
    'open', 'close', 'word' or 'other', and start the token's offset. Comments
    are skipped; depth counts the open {, [ and ( around the token (an opening
    token carries the depth outside it). Interpolations stay inside their string."""
    i, n, depth = 0, len(text), 0
    while i < n:
        c = text[i]
        if c == "#" or text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j + 1
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            i = n if j < 0 else j + 2
        elif text.startswith("<<", i) and re.match(r"<<-?([A-Za-z_][A-Za-z0-9_-]*)\r?\n", text[i:]):
            m = re.match(r"<<-?([A-Za-z_][A-Za-z0-9_-]*)\r?\n", text[i:])
            end = re.compile(r"^[ \t]*" + re.escape(m.group(1)) + r"[ \t]*$", re.M)
            body_start = i + m.end()
            e = end.search(text, body_start)
            body = text[body_start : e.start() if e else n]
            yield ("string", body, depth, i)
            i = e.end() if e else n
        elif c == '"':
            j, buf, nest = i + 1, [], 0
            while j < n:
                d = text[j]
                if d == "\\" and j + 1 < n:
                    buf.append(text[j : j + 2])
                    j += 2
                    continue
                if text.startswith("${", j) or text.startswith("%{", j):
                    nest += 1
                    buf.append(text[j : j + 2])
                    j += 2
                    continue
                if d == "}" and nest:
                    nest -= 1
                elif d == '"' and not nest:
                    break
                buf.append(d)
                j += 1
            yield ("string", "".join(buf), depth, i)
            i = j + 1
        elif c in "{[(":
            yield ("open", c, depth, i)
            depth += 1
            i += 1
        elif c in "}])":
            depth = max(depth - 1, 0)
            yield ("close", c, depth, i)
            i += 1
        elif c.isalnum() or c in "_-":
            m = re.match(r"[A-Za-z0-9_.-]+", text[i:])
            yield ("word", m.group(0), depth, i)
            i += m.end()
        elif c.isspace():
            i += 1
        else:
            yield ("other", c, depth, i)
            i += 1


def sensitive_variables(tf_files):
    """Names of the `variable "<n>" {` blocks in tf_files that set `sensitive = true`."""
    names = set()
    for f in tf_files:
        toks = list(_tokens(Path(f).read_text(encoding="utf-8", errors="replace")))
        k = 0
        while k < len(toks):
            kind, val, depth, _ = toks[k]
            if (
                kind == "word"
                and val == "variable"
                and depth == 0
                and k + 2 < len(toks)
                and toks[k + 1][0] == "string"
                and toks[k + 2][:2] == ("open", "{")
            ):
                name = toks[k + 1][1]
                k += 3
                # Attributes of the block itself sit at depth 1.
                while k < len(toks) and not (toks[k][0] == "close" and toks[k][2] == 0):
                    t = toks[k]
                    if (
                        t[:2] == ("word", "sensitive")
                        and t[2] == 1
                        and k + 2 < len(toks)
                        and toks[k + 1][:2] == ("other", "=")
                        and toks[k + 2][:2] == ("word", "true")
                    ):
                        names.add(name)
                    k += 1
            k += 1
    return names


def assignments(tfvars_text):
    """Each top-level `name = value` in tfvars_text, with the value's raw text."""
    toks = list(_tokens(tfvars_text))
    names = [
        (toks[k][1], toks[k + 1][3] + 1, toks[k][3])
        for k in range(len(toks) - 1)
        if toks[k][0] == "word"
        and toks[k][2] == 0
        and toks[k + 1][:2] == ("other", "=")
        and not (k + 2 < len(toks) and toks[k + 2][:2] == ("other", "=") and toks[k + 2][3] == toks[k + 1][3] + 1)
    ]
    result = {}
    for idx, (name, value_start, _) in enumerate(names):
        value_end = names[idx + 1][2] if idx + 1 < len(names) else len(tfvars_text)
        result[name] = tfvars_text[value_start:value_end].strip()
    return result


def _matches(path, pattern):
    return fnmatch.fnmatchcase(PurePosixPath(path).name, pattern)


def findings(root):
    """Every finding (a) to (d) for the git repository at root, one line each."""
    root = Path(root)
    files = tracked_files(root)
    found = []

    # (a) secrets, state or provider caches tracked.
    for p in files:
        parts = PurePosixPath(p).parts
        if (
            _matches(p, SECRET_FILE)
            or _matches(p, "*.tfstate")
            or _matches(p, "*.tfstate.*")
            or ".terraform" in parts[:-1]
        ):
            found.append(f"{p}: is tracked by git; secrets and state must stay out of it (git rm --cached {p})")

    # (b) a committed tfvars file setting a sensitive variable.
    sensitive = sensitive_variables(
        [root / p for p in files if p.endswith(".tf") and (root / p).is_file()]
    )
    for p in files:
        if not _matches(p, "*.tfvars") or _matches(p, SECRET_FILE) or not (root / p).is_file():
            continue
        text = (root / p).read_text(encoding="utf-8", errors="replace")
        for name in sorted(set(assignments(text)) & sensitive):
            found.append(
                f"{p}: sets the sensitive variable {name}; set it only in terraform.sensitive.auto.tfvars"
            )

    # (c) the shipped example holds placeholders only.
    for p in files:
        if not _matches(p, EXAMPLE_FILE) or not (root / p).is_file():
            continue
        text = (root / p).read_text(encoding="utf-8", errors="replace")
        for kind, val, _, _ in _tokens(text):
            if kind == "string" and val != "" and not PLACEHOLDER.match(val):
                found.append(f'{p}: holds the value "{val}"; an example holds only "" or "<placeholder>" values')

    # (d) .gitignore keeps covering secrets and state.
    gitignore = root / ".gitignore"
    lines = (
        {line.strip() for line in gitignore.read_text(encoding="utf-8").splitlines()}
        if gitignore.is_file()
        else set()
    )
    for pattern in GITIGNORE_PATTERNS:
        if pattern not in lines:
            found.append(f".gitignore: does not list {pattern}")

    return found


def main(argv):
    root = argv[1] if len(argv) > 1 else "."
    found = findings(root)
    for line in found:
        print(line)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
