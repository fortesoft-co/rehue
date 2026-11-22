# Nix and Stylix

```nix
inputs.rehue.url = "github:fortesoft-co/rehue";
```

The flake provides the package, a devshell with the pinned toolchain
(`nix develop`), and builders for wiring rehue into your config.

## Stylix

The Nix builders run the whole pipeline inside the build sandbox. The
mapped scheme feeds `stylix.base16Scheme`, the repainted wallpaper
feeds `stylix.image`, and GTK, Firefox, Zed, terminals and GNOME all
restyle from one store path. Nothing reads global state and nothing
touches the network. The outputs (scheme YAML, PNG, extraction report)
commit and diff like the rest of your config, so your scheme and the
picture it came from can never drift apart.

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

The builders take the same settings as the commands: `config` is the
`--config` JSON as a Nix attribute set ([config.md](config.md)).

## Icon themes

`lib.icons` recolors a theme from any
icon package and gives back a package for `stylix.icons`:

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
The preview sheet is in `${icons}/share/rehue/preview.png`. Inside a
Nix build only built-in looks are available; a look of your own works
by path (`config.look = "${./dusk.toml}";`).
