# Query block, format 1

A query block is a fenced code block whose info string is `katalogr-query`. Its body is one
line: an operation, then its arguments and flags. A tool that does not know the block shows
its source as written.

Each block below has a heading above it, `valid` or `invalid`. A reader that follows this
file accepts every valid block and rejects every invalid one.

Rules:

- The fence is three or more backticks or tildes, indented at most three spaces. A fence
  inside a longer or different fence is content, not a block. An unclosed fence is not a block.
- The body is exactly one line. An empty body, or a body of two lines, is invalid.
- The operations are `structure`, `contracts`, `callers`, `flow`, `impact`, `lineage`, `path`, `process` and `cycles`. Any other word
  is invalid.
- A flag is `--name value` or `--name=value`. A switch may stand alone, or take `true` or
  `false`. A flag may be given once. An unknown flag is invalid.
- Positional arguments come in the operation's own number: none for `structure`, `contracts`
  and `cycles`, one for `callers`, `flow`, `impact`, `lineage` and `process` (a symbol), two for `path` (from, to). More or fewer is
  invalid. A word with spaces is wrapped in double quotes.
- A number is a whole number above 0. A word flag takes one of its listed words.

## Valid blocks

### valid

```katalogr-query
structure
```

### valid

```katalogr-query
structure --level file --max-nodes 40 --inferred
```

### valid

```katalogr-query
contracts
```

### valid

```katalogr-query
contracts --kind queue --max-nodes=20
```

### valid

```katalogr-query
callers parse_config --depth 3
```

### valid

```katalogr-query
callers "Config load" --max-nodes 10
```

### valid

```katalogr-query
impact parse_config --depth 2 --max-nodes 40
```

### valid

```katalogr-query
lineage Order --depth 2 --max-nodes 40
```

### valid

```katalogr-query
flow handle_order --depth 4 --max-nodes 40 --reverse
```

### valid

```katalogr-query
path start finish --max-len 4
```

### valid

```katalogr-query
process order --max-nodes 100
```

### valid

```katalogr-query
cycles --level file --limit 3
```

### valid

~~~katalogr-query
cycles
~~~

## Invalid blocks

### invalid

```katalogr-query
```

### invalid

```katalogr-query
unknown-op
```

### invalid

```katalogr-query
structure --no-such-flag 1
```

### invalid

```katalogr-query
structure --level galaxy
```

### invalid

```katalogr-query
structure --max-nodes 0
```

### invalid

```katalogr-query
structure --max-nodes
```

### invalid

```katalogr-query
structure --level file --level package
```

### invalid

```katalogr-query
callers
```

### invalid

```katalogr-query
flow
```

### invalid

```katalogr-query
flow handle_order --depth 0
```

### invalid

```katalogr-query
path only-one
```

### invalid

```katalogr-query
cycles extra
```

### invalid

```katalogr-query
structure
cycles
```

### invalid

```katalogr-query
callers "unterminated
```
