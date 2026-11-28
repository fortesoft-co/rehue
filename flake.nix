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
            version = "0.6.0";
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
            meta = with pkgs.lib; {
              description = "Map base16 color schemes and wallpapers onto each other";
              license = licenses.agpl3Plus;
              mainProgram = "rehue";
            };
          };

          # the soft and general models for `rehue enhance --sr`, built
          # from the official Real-ESRGAN weights (BSD-3-Clause, Xintao
          # Wang) by scripts/sr-models.py; point --sr-models,
          # $REHUE_SR_MODELS or sr-models in a config at it.  The same
          # files ship as rehue-sr-models.tar.gz on each release.
          sr-models =
            let
              weight =
                release: name: hash:
                pkgs.fetchurl {
                  url = "https://github.com/xinntao/Real-ESRGAN/releases/download/${release}/${name}";
                  inherit hash;
                };
              python = pkgs.python3.withPackages (p: [
                p.torch
                p.numpy
              ]);
            in
            pkgs.runCommand "rehue-sr-models" { nativeBuildInputs = [ python ]; } ''
              mkdir weights
              ln -s ${weight "v0.1.1" "RealESRNet_x4plus.pth" "sha256-qCC5veiah011mdVFVnMIzmwSj8h1SlMgjtoBbUCqgd8="} weights/RealESRNet_x4plus.pth
              ln -s ${weight "v0.2.5.0" "realesr-general-x4v3.pth" "sha256-jcftuayAzNwww6XcpmFlCTZ/BfvBhK2VtzHwW+zpYpI="} weights/realesr-general-x4v3.pth
              ln -s ${weight "v0.2.5.0" "realesr-general-wdn-x4v3.pth" "sha256-FkH4xEZLnwl8n92lWJJzcT9nz1nz2QngvWiPDO4mnco="} weights/realesr-general-wdn-x4v3.pth
              python3 -I ${./scripts/sr-models.py} weights "$out"
            '';
        in
        {
          inherit rehue sr-models;
          default = rehue;
        }
      );

      apps = forAllSystems (pkgs: {
        rehue = {
          type = "app";
          program = "${self.packages.${pkgs.stdenv.hostPlatform.system}.rehue}/bin/rehue";
        };
        default = self.apps.${pkgs.stdenv.hostPlatform.system}.rehue;
      });

      devShells = forAllSystems (pkgs: rec {
        default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustc
            cargo
            clippy
            rustfmt
            rust-analyzer
            # video wallpapers: wal drives ffmpeg, and the video tests
            # skip without it
            ffmpeg-headless
          ];
        };

        # Everything the docs image builder needs on top of the
        # toolchain: imagemagick for the montages, libwebp's img2webp for
        # the animated video examples, headless chromium for
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
              libwebp
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
          rehue = self.packages.${pkgs.stdenv.hostPlatform.system}.rehue;
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

          # repaint a wallpaper with a scheme's palette (`rehue wal`); a
          # video (.mp4, .webm, .mkv, .mov) builds with ffmpeg
          wal =
            { wallpaper, scheme, config ? { } }:
            let
              configFile = pkgs.writeText "rehue-wal-config.json" (builtins.toJSON config);
              name = pkgs.lib.toLower (toString wallpaper);
              isVideo = builtins.any (ext: pkgs.lib.hasSuffix ext name) [
                ".mp4"
                ".webm"
                ".mkv"
                ".mov"
              ];
            in
            pkgs.runCommand "rehue-wal" {
              nativeBuildInputs = [ rehue ] ++ pkgs.lib.optional isVideo pkgs.ffmpeg-headless;
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

          # scale a picture or video up (`rehue enhance`, Lanczos: the
          # sandbox has no GPU for --sr); the result is
          # $out/wallpaper.png, or $out/wallpaper.<ext> for a video
          enhance =
            { wallpaper, config }:
            let
              configFile = pkgs.writeText "rehue-enhance-config.json" (builtins.toJSON config);
              name = pkgs.lib.toLower (toString wallpaper);
              video = pkgs.lib.findFirst (ext: pkgs.lib.hasSuffix ".${ext}" name) null [
                "mp4"
                "webm"
                "mkv"
                "mov"
              ];
            in
            pkgs.runCommand "rehue-enhance" {
              nativeBuildInputs = [ rehue ] ++ pkgs.lib.optional (video != null) pkgs.ffmpeg-headless;
              wallpaperArg = "${wallpaper}";
              configArg = configFile;
            } ''
              rehue enhance \
                --input "$wallpaperArg" \
                --config "$configArg" \
                --out build
              mkdir -p "$out"
              mv build/* "$out/wallpaper.${if video != null then video else "png"}"
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