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
            version = "0.1.0";
            src = self;
            cargoLock = {
              lockFile = ./Cargo.lock;
              # The tinted-schemes git dependency (a packaging branch of
              # the scheme collection) needs its vendored content hash.
              outputHashes."tinted-schemes-0.1.0" =
                "sha256-OVOQASFtYhkXkoChfVp9o9LwFEXF9wQmpXa0+MCNdHs=";
            };
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

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustc
            cargo
            clippy
            rustfmt
            rust-analyzer
          ];
        };
      });

      # Derivation builders a consumer flake can import directly, e.g.:
      #   mapped = rehue.lib.${system}.mapScheme {
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
          mapScheme =
            { wallpaper, scheme, config ? { } }:
            let
              configFile = pkgs.writeText "rehue-map-config.json" (builtins.toJSON config);
            in
            pkgs.runCommand "rehue-mapped-scheme" {
              nativeBuildInputs = [ rehue ];
              wallpaperArg = "${wallpaper}";
              schemeArg = "${scheme}";
              configArg = configFile;
            } ''
              rehue map-scheme \
                --wallpaper "$wallpaperArg" \
                --scheme "$schemeArg" \
                --config "$configArg" \
                --out "$out"
            '';

          mapWal =
            { wallpaper, scheme, config ? { } }:
            let
              configFile = pkgs.writeText "rehue-map-wal-config.json" (builtins.toJSON config);
            in
            pkgs.runCommand "rehue-mapped-wallpaper" {
              nativeBuildInputs = [ rehue ];
              wallpaperArg = "${wallpaper}";
              schemeArg = "${scheme}";
              configArg = configFile;
            } ''
              rehue map-wal \
                --wallpaper "$wallpaperArg" \
                --scheme "$schemeArg" \
                --config "$configArg" \
                --out "$out"
            '';
        }
      );
    };
}