# Config files

Every flag has a config key, so a grade you want to keep can live in a
file passed with `-c` (`--config`). Keys are the flag names without
the dashes. A config can be TOML, YAML or JSON; the file's extension
(`.toml`, `.yaml` or `.yml`, `.json`) picks the format:

```toml
# grade.toml
harmonize = 1
reach = 90
palette = { ac = 1, base0D = 2 }
```

```yaml
# grade.yaml
harmonize: 1
reach: 90
palette: { ac: 1, base0D: 2 }
```

```json
{ "harmonize": 1, "reach": 90, "palette": { "ac": 1, "base0D": 2 } }
```

```sh
rehue wal -i wall.png -s gruvbox-light -c grade.toml
```

YAML is read as YAML 1.2, so a bare `no` or `on` stays a word rather
than turning into true or false.

Every key is optional: anything left out keeps its default. A few
settings exist only as keys: the [extraction](grading.md#extraction)
settings, and the `folders` color for [icons](icons.md) (which also
has `--folders`). Unknown keys are errors, with a hint when one looks
like a typo.

Per-role settings sit under `roles`, one record per role (full or
short names) plus `all` for every role. On `wal` and `icons`, the
role records grade pixels by the role of the slot they land on:

```toml
[roles.all]
match-hue = 1

[roles.accents]
chroma = 1.3

[roles.bg]
lightness = -0.05
```

The [`apply` pipeline](apply.md) takes the same keys in its
`[rehue.scheme]` and `[rehue.wal]` tables, and [looks](looks.md) in
their sections.

## Merge and precedence

Settings layer in this order, each one winning over the ones before
it, key by key:

1. the defaults,
2. the looks (`-l`, else the config's `look` key), left to right,
3. the `--config` file (or the pipeline table),
4. the flags.

Within each layer, config records resolve `globals // all // role`: bare flags seed
`all`, flags with a role (`--chroma bg 1.6`) write that role's record,
and `--config` records do the same job, with one precedence rule
per key:

1. the role's own record (flag with a role, or config file),
2. else the `all` seed (bare flag or config `all` record),
3. else the role's default.

Only wal's bare `--lightness` / `--chroma` differ by design: they are
the image-wide grade, not a seed. Extraction options are config-file
only. `dither` is global-only: role records do not carry it. Role
names take their short forms in config keys too (`"bg"`, `"ac"`).
`palette` is the one key that replaces instead of merging (a
`--dither` flag likewise replaces the config's `dither`): a palette
in a later layer replaces the earlier one whole. Unknown keys are
errors, in configs, looks and pipelines alike.

The workshop example, expressed twice:

```sh
# flags form
rehue scheme -i solarized-dark -w pic.webp -o out \
  --match-hue 1 \
  --chroma bg 1.6 --chroma sf 1.6 --chroma fg 1.6 --chroma ac 0.75 \
  --assign bg 1 --assign sf 1,3 --assign fg 0 \
  --assign ac 0,1,2,3
```

```toml
[roles.all]
match-hue = 1.0

[roles.background]
assign = 1
chroma = 1.6

[roles.surfaces]
assign = [1, 3]
chroma = 1.6

[roles.foreground]
assign = 0
chroma = 1.6

[roles.accents]
assign = [0, 1, 2, 3]
chroma = 0.75
```

Both run the same merge and the same engine; the config file wins when
the same dial needs the same value on several roles.
