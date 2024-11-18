{
  nixConfig = {
    extra-substituters = [ "https://cache.garnix.io" ];
    extra-trusted-public-keys =
      [ "cache.garnix.io:CTFPyKSLcx5RMJKfLo5EEPUObbA78b0YQ2DTCJXqr9g=" ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { nixpkgs, crane, ... }:
    let
      forEachSystem = fn: with nixpkgs.lib;
        zipAttrsWith (_: mergeAttrsList) (map fn systems.flakeExposed);
    in
    forEachSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        craneLib = crane.mkLib pkgs;

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
