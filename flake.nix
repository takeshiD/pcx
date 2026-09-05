{
  description = "pcx — a shell-native point-cloud toolbox";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      ...
    }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      pkgsFor =
        system:
        import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rustToolchain;
            rustc = rustToolchain;
          };
          pcx = rustPlatform.buildRustPackage {
            pname = "pcx-cli";
            version = "0.1.1";
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
            doCheck = true;
            nativeBuildInputs = [ pkgs.installShellFiles ];

            postInstall = ''
              installShellCompletion \
                --bash generated/completions/pcx.bash \
                --zsh generated/completions/_pcx \
                --fish generated/completions/pcx.fish
              installManPage generated/man/*.1
            '';

            meta = {
              description = "Shell-native point-cloud toolbox for edge Linux systems";
              homepage = "https://github.com/takeshiD/pcx";
              license = pkgs.lib.licenses.mit;
              mainProgram = "pcx";
              platforms = supportedSystems;
            };
          };
        in
        {
          default = pcx;
          pcx = pcx;
        }
      );

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = "${self.packages.${system}.default}/bin/pcx";
          meta.description = "Run the pcx command-line tool";
        };
      });

      checks = forAllSystems (system: {
        package = self.packages.${system}.default;
      });

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          fuzzToolchain = pkgs.rust-bin.nightly.latest.default.override {
            extensions = [ "rust-src" ];
          };
        in
        {
          default = pkgs.mkShell {
            packages = [
              rustToolchain
              pkgs.cargo-fuzz
              pkgs.nodejs_24
              pkgs.nixfmt
              pkgs.python3
            ];
          };

          fuzz = pkgs.mkShell {
            packages = [
              fuzzToolchain
              pkgs.cargo-fuzz
            ];
          };
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt);
    };
}
