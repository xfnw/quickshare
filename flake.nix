{
  inputs = {
    nixpkgs.url = "https://channels.nixos.org/nixpkgs-unstable/nixexprs.tar.zst";
    naersk = {
      url = "github:nix-community/naersk";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, naersk, nixpkgs }: let
    forEachSystem = nixpkgs.lib.genAttrs [
      "aarch64-darwin" 
      "aarch64-linux"
      "i686-linux"
      "riscv64-linux"
      "x86_64-linux"
    ];
  in {
    packages = forEachSystem (system: let
      pkgs = nixpkgs.legacyPackages.${system};
      naersk' = pkgs.callPackage naersk { };
    in {
      quickshare = naersk'.buildPackage { src = ./.; };
      default = self.packages.${system}.quickshare;
    });

    devShells = forEachSystem (system: let
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      default = pkgs.mkShell {
        packages = with pkgs; [
          rustc cargo clippy
        ];
      };
    });
  };
}
