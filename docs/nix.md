# Nix and Stylix

```nix
inputs.rehue.url = "github:fortesoft-co/rehue";
```

The flake provides the package (`rehue.packages.${system}.default`), a
devshell with the pinned toolchain (`nix develop`), and builders that
run rehue inside a Nix build, so its results are store paths your
config can use.

## Builders

Each builder runs one command in the build sandbox. Nothing reads
global state and nothing touches the network, so the outputs (scheme,
wallpaper, report) build the same way every time and your scheme and
the picture it came from can't drift apart.

| builder | runs | takes | gives |
| --- | --- | --- | --- |
| `lib.${system}.scheme` | `rehue scheme` | `wallpaper`, `scheme`, `config` | `scheme.yaml`, `clusters.json`, `preview.png`, `preview.html` |
| `lib.${system}.wal` | `rehue wal` | `wallpaper`, `scheme`, `config` | `wallpaper.png` (or `.mp4`, `.webm`, `.mkv`, `.mov` for a video), `compare.png`, `report.json`, `preview.html`, a LUT when `config.lut` is set |
| `lib.${system}.icons` | `rehue icons` | `icons`, `theme`, `scheme`, `name`, `config` | a package with `share/icons/<name>`, and `share/rehue/preview.png` |
| `lib.${system}.enhance` | `rehue enhance` | `wallpaper`, `config` | `wallpaper.png` |

- `wallpaper` and `scheme` are paths (a scheme can also be a bundled
  name, such as `"gruvbox-light"`).
- `config` is a [config file](config.md) as a Nix attribute set: every
  setting a flag has, by the flag's name without the dashes. It is
  optional except for `enhance`, which needs an `upscale`.
- A video wallpaper builds with ffmpeg (nixpkgs' ffmpeg-headless), added when
  the `wallpaper` path ends in `.mp4`, `.webm`, `.mkv` or `.mov`.
- `enhance` scales with Lanczos: the sandbox has no GPU for `sr`.
- Inside a build only built-in looks are available; a look of your own
  works by path (`config.look = "${./dusk.toml}";`).

## Stylix

The scheme `lib.scheme` maps feeds `stylix.base16Scheme` and the repainted
wallpaper feeds `stylix.image`, so GTK, Firefox, Zed, terminals and
GNOME all restyle from one store path:

```nix
# in a module:
let
  mapped = rehue.lib.${system}.scheme {
    wallpaper = ./assets/glitter.webp;
    scheme = "${pkgs.base16-schemes}/share/themes/gruvbox-light.yaml";
    # config = { roles.foreground = { match-hue = 0.4; chroma = 0.85; }; };
  };
  repainted = rehue.lib.${system}.wal {
    wallpaper = ./assets/glitter.webp;
    scheme = "${mapped}/scheme.yaml";
    config = { harmonize = 1.0; };
  };
in {
  stylix.base16Scheme = "${mapped}/scheme.yaml";
  stylix.image = "${repainted}/wallpaper.png";
}
```

## A video wallpaper

`lib.wal` takes a video the same way, with every video setting as a
key ([video.md](video.md)). Stylix wants a still for `stylix.image`;
hand the video to your player instead:

```nix
let
  looped = rehue.lib.${system}.wal {
    wallpaper = ./assets/rain.mp4;
    scheme = "kanagawa";
    config = {
      harmonize = 1;
      deband = true;
      grain = true;
      loop = { mode = "fade"; min = "50%"; };
    };
  };
in {
  # hand it to your player, e.g. in a session startup script:
  # mpvpaper -p -o "loop" DP-1 ${looped}/wallpaper.mp4
}
```

`--loop ask` asks at a terminal, so in a build pick a loop with
`mode = "auto"` (the default) or pin the one `ask` showed you, using
the flag it printed (`loop = "fade,start=12.533,end=21.467";`).

## A bigger picture first

`lib.enhance` scales a small picture up before it's graded:

```nix
let
  bigger = rehue.lib.${system}.enhance {
    wallpaper = ./assets/small.png;
    config = { upscale = 2; };
  };
  repainted = rehue.lib.${system}.wal {
    wallpaper = "${bigger}/wallpaper.png";
    scheme = "nord";
  };
in { stylix.image = "${repainted}/wallpaper.png"; }
```

## Icon themes

`lib.icons` recolors a theme from any icon package and gives back a
package for `stylix.icons`:

```nix
let
  icons = rehue.lib.${system}.icons {
    icons = pkgs.papirus-icon-theme;
    theme = "Papirus-Dark";
    scheme = "${mapped}/scheme.yaml";
    name = "Papirus-Rehue";
    # config = { folders = { base0E = 1; }; };   # the folder color
  };
in {
  stylix.icons = {
    enable = true;
    package = icons;
    dark = "Papirus-Rehue";
    light = "Papirus-Rehue";
  };
}
```

Set both `dark` and `light`: Stylix's GTK and Qt targets each pick one.
