{
  description = "rehue - map base16 color schemes and wallpapers onto each other";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs [
          "x86_64-linux"
          "aarch64-linux"
          "x86_64-darwin"
          "aarch64-darwin"
        ] (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (
        pkgs:
        let
          rehue = pkgs.rustPlatform.buildRustPackage {
            pname = "rehue";
            version = "0.3.0";
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
            meta = with pkgs.lib; {
              description = "Map base16 color schemes and wallpapers onto each other";
              license = licenses.agpl3Plus;
              mainProgram = "rehue";
            };
          };
        in
        {
          inherit rehue;
          default = rehue;
        }
      );

      apps = forAllSystems (pkgs: {
        rehue = {
          type = "app";
          program = "${self.packages.${pkgs.system}.rehue}/bin/rehue";
        };
        default = self.apps.${pkgs.system}.rehue;
      });

      devShells = forAllSystems (pkgs: rec {
        default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustc
            cargo
            clippy
            rustfmt
            rust-analyzer
          ];
        };

        # Everything the docs image builder needs on top of the
        # toolchain: imagemagick for the montages, headless chromium for
        # the preview.html screenshots, Papirus for the icons example
        # (kept out of `default` so a plain `cargo test` shell doesn't
        # pull a browser).
        #   nix develop .#examples -c cargo run --release --example docs-images
        examples = pkgs.mkShell {
          inputsFrom = [ default ];
          buildInputs =
            with pkgs;
            [
              imagemagick
              dejavu_fonts
            ]
            ++ lib.optionals stdenv.hostPlatform.isLinux [ chromium ];
          FONTCONFIG_FILE = pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; };
          # the only icon themes the examples see, so `-i Papirus-Dark`
          # finds the pinned Papirus whatever the machine has installed
          REHUE_DOCS_ICONS = "${pkgs.papirus-icon-theme}/share";
        };
      });

      # Derivation builders a consumer flake can import directly, e.g.:
      #   mapped = rehue.lib.${system}.scheme {
      #     wallpaper = ./wallpaper.png;
      #     scheme = "${pkgs.base16-schemes}/share/themes/rose-pine.yaml";
      #   };
      #   stylix.base16Scheme = "${mapped}/scheme.yaml";
      lib = forAllSystems (
        pkgs:
        let
          rehue = self.packages.${pkgs.system}.rehue;
        in
        {
          # rehue a scheme with a wallpaper's hues (`rehue scheme`)
          scheme =
            { wallpaper, scheme, config ? { } }:
            let
              configFile = pkgs.writeText "rehue-scheme-config.json" (builtins.toJSON config);
            in
            pkgs.runCommand "rehue-scheme" {
              nativeBuildInputs = [ rehue ];
              wallpaperArg = "${wallpaper}";
              schemeArg = "${scheme}";
              configArg = configFile;
            } ''
              rehue scheme \
                --input "$schemeArg" \
                --wallpaper "$wallpaperArg" \
                --config "$configArg" \
                --out "$out"
            '';

          # repaint a wallpaper with a scheme's palette (`rehue wal`)
          wal =
            { wallpaper, scheme, config ? { } }:
            let
              configFile = pkgs.writeText "rehue-wal-config.json" (builtins.toJSON config);
            in
            pkgs.runCommand "rehue-wal" {
              nativeBuildInputs = [ rehue ];
              wallpaperArg = "${wallpaper}";
              schemeArg = "${scheme}";
              configArg = configFile;
            } ''
              rehue wal \
                --input "$wallpaperArg" \
                --scheme "$schemeArg" \
                --config "$configArg" \
                --out "$out"
            '';

          # recolor an icon theme with a scheme (`rehue icons`); the result
          # is a package with share/icons/<name>, for stylix.icons.package
          icons =
            {
              icons,
              theme,
              scheme,
              name ? "${theme}-rehue",
              config ? { },
            }:
            let
              configFile = pkgs.writeText "rehue-icons-config.json" (builtins.toJSON config);
            in
            pkgs.runCommand "rehue-icons" {
              nativeBuildInputs = [
                rehue
                pkgs.gtk3 # gtk-update-icon-cache
              ];
              themeArg = "${icons}/share/icons/${theme}";
              schemeArg = "${scheme}";
              nameArg = name;
              configArg = configFile;
            } ''
              rehue icons \
                --input "$themeArg" \
                --scheme "$schemeArg" \
                --name "$nameArg" \
                --config "$configArg" \
                --out build
              mkdir -p "$out/share/icons" "$out/share/rehue"
              mv build/*/ "$out/share/icons/"
              mv build/preview.png "$out/share/rehue/"
            '';
        }
      );
    };
}