{
  nixConfig = {
    extra-substituters = [ "https://cache.garnix.io" ];
    extra-trusted-public-keys =
      [ "cache.garnix.io:CTFPyKSLcx5RMJKfLo5EEPUObbA78b0YQ2DTCJXqr9g=" ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    fenix.url = "github:nix-community/fenix";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { nixpkgs, fenix, crane, ... }:
    let
      forEachSystem = fn: with nixpkgs.lib;
        zipAttrsWith (_: mergeAttrsList) (map fn systems.flakeExposed);
    in
    forEachSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        toolchain = fenix.packages.${system}.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-yMuSb5eQPO/bHv+Bcf/US8LVMbf/G/0MSfiPwBhiPpk=";
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;

        commonArgs = {
          src = craneLib.cleanCargoSource ./.;         
          buildInputs = with pkgs; lib.optionals (stdenv.hostPlatform.isDarwin) [
            iconv
          ];
        };

        depsArtifacts = craneLib.buildDepsOnly commonArgs;

        nu_plugin_httpserve = craneLib.buildPackage (commonArgs // {
          cargoArtifacts = depsArtifacts;
        });
      in {
        packages.${system} = {
          default = nu_plugin_httpserve;
          inherit nu_plugin_httpserve;
        };

        devShells.${system} = {
          default = craneLib.devShell {
          };
        };
      }
    );
}
