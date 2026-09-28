# ears-classifier

Classify a requirement by its **EARS** pattern, and canonicalise **ISO/IEC 25010** quality
characteristics. Text in, enum out — no I/O, no configuration, one small dependency.

```rust
use ears_classifier::{classify, normalize_iso, Pattern, ISO25010};

assert_eq!(
    classify("WHEN the archive is downloaded, THE SYSTEM SHALL verify its checksum"),
    Some(Pattern::Event)
);
assert_eq!(classify("it should probably be fast"), None);

assert_eq!(normalize_iso("performance-efficiency"), Some("Performance Efficiency"));
assert_eq!(normalize_iso("Speed"), None);
assert_eq!(ISO25010.len(), 9);
```

## EARS

[Easy Approach to Requirements Syntax](https://alistairmavin.com/ears/) gives a requirement a shape
that can be tested: one behaviour, one trigger, no compound sentences. The five patterns:

| Pattern | Shape |
|---|---|
| `Ubiquitous` | `THE SYSTEM SHALL <response>` |
| `Event` | `WHEN <trigger>, THE SYSTEM SHALL <response>` |
| `State` | `WHILE <state>, THE SYSTEM SHALL <response>` |
| `Optional` | `WHERE <feature>, THE SYSTEM SHALL <response>` |
| `Unwanted` | `IF <condition>, THEN THE SYSTEM SHALL <response>` |

## It classifies; it never rejects

`classify` returns `None` for text that fits no pattern, and that is the whole contract. Nothing
here refuses a write or fails a parse.

That is deliberate. A tool that rejected a requirement over its prose style would teach people to
stop recording requirements — so the caller decides what to do with a `None`: warn, report, or
ignore. Deriving the pattern on demand also means stored text can never disagree with the
classifier, which it would if the pattern were written down beside it.

**Deliberately not checked:** compound responses (`SHALL validate and reject`). A regex for that
fires on legitimate single behaviours, and a check people learn to ignore is worse than no check.

## ISO/IEC 25010

`normalize_iso` maps a tag to its canonical spelling, ignoring case, spacing and punctuation, and
returns `None` for anything outside the nine characteristics of the 2023 revision. Same contract:
it reports, it does not reject.

## Where it comes from

Extracted from [kanbanr](https://github.com/startr-trade/kanbanr), which uses it to check that every
requirement on a board states one testable behaviour and that a quality claim names a real
characteristic. It has no dependency on kanbanr and is useful anywhere requirements are written
down.

## License

MIT OR Apache-2.0.
