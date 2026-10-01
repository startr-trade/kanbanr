# The kanbanr Claude Code plugin

`plugin.json` and `marketplace.json` make this repository a one-plugin Claude Code marketplace:

```bash
claude plugin marketplace add startr-trade/kanbanr
claude plugin install kanbanr@kanbanr
```

The plugin bundles the skill (`skill/kanbanr/SKILL.md`) and three of its hooks — session start,
the stop reminder and the session summaries — with `${CLAUDE_PLUGIN_ROOT}`-relative commands, so
they work from wherever Claude Code caches the plugin. The commit and docs guards are registered
per project by `kanbanr hooks install`. The `kanbanr` program itself is not in the plugin: install
it from the GitHub release.

Notes for editing the manifests:

- `skills` takes paths relative to the plugin root (`./skill/`), not `${CLAUDE_PLUGIN_ROOT}`, which
  is expanded only in hook commands. The skill lives under the singular `skill/`, so it is named
  explicitly.
- The manifests are strict JSON with no comment keys; this file is where the notes live.
- `claude plugin validate .` must pass with no errors or warnings; `make ci` runs it wherever the
  `claude` command is installed (FEAT-139).
