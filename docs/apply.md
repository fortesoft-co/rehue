# apply: run a pipeline and theme your desktop

`rehue apply` runs a whole pipeline from one TOML file and hands the
result to your desktop. rehue makes the colors and the wallpaper;
[tinty](https://github.com/tinted-theming/tinty) renders the scheme
into every app you've set up in it (terminals, shells, editors, bars),
using the tinted-theming templates.

**Prerequisite:** a working tinty setup. rehue hands tinty the scheme,
but which apps get themed, and how each one picks up its theme (an
import line in the app's config, or a hook), is tinty's configuration.
See tinty's [README](https://github.com/tinted-theming/tinty#readme)
and [USAGE](https://github.com/tinted-theming/tinty/blob/main/USAGE.md)
to set it up; each template repo's README shows the matching config
line or hook for its apps.

```toml
# ~/.config/rehue/pipeline.toml
wallpaper = "~/Pictures/glitter.webp"
scheme = "gruvbox-light"        # a bundled name, or a path to yaml
# out = "~/.local/share/rehue/current"   (the default)

[rehue]
look = "pop"                    # both steps start from a look (optional)

[rehue.scheme]                  # rehue the scheme (empty = the look / defaults)
[rehue.scheme.roles.accents]
chroma = 1.2

[rehue.wal]                     # repaint the wallpaper with it
harmonize = 1.0

[apply]
wallpaper-command = "swww img {wallpaper}"
```

```sh
rehue apply --pipeline ~/.config/rehue/pipeline.toml -n   # preview
rehue apply --pipeline ~/.config/rehue/pipeline.toml
```

- **Steps:** `[rehue.scheme]` and `[rehue.wal]` take the same keys as their
  `--config` files ([config.md](config.md)); a table being present turns that step on.
  With both, the wallpaper is repainted with the scheme it just
  produced (the round trip). With only `[rehue.wal]`, the reference
  scheme is applied as-is. `look` under `[rehue]` starts both steps
  from a look; a step's own `look` key replaces it, and the table's
  keys win over either.
- **tinty:** the scheme is installed as a tinty custom scheme
  (`<tinty data dir>/custom-schemes/base16/rehue.yaml`) and applied
  with `tinty apply base16-rehue`; tinty then builds and installs the
  themes and runs your hooks. Needs **tinty 0.35 or newer** (older
  versions can't build themes for custom schemes). `tinty-name`
  changes the name, `tinty = false` skips tinty, and `tinty-config` /
  `tinty-data-dir` point it at another setup (handy for trying things
  without touching your real one).
- **Wallpaper:** `wallpaper-command` runs through `sh` with
  `{wallpaper}` replaced by the (quoted) path of the image (or video,
  when the pipeline's wallpaper is one, for a player such as mpvpaper); `$REHUE_WALLPAPER`
  and `$REHUE_SCHEME` are set too. Any setter works: swww, swaybg,
  feh, `gsettings set org.gnome.desktop.background picture-uri ...`.
- **Paths** in the file resolve against the file's directory (`~/`
  expands), and commands run from there. Outputs (`scheme/`,
  `wallpaper/`, previews) land in `out`.
