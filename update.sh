#!/usr/bin/env nix-shell
#!nix-shell -i bash -p bash curl jq gnused coreutils nix

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
PACKAGE_NIX="$SCRIPT_DIR/default.nix"

OWNER="QingJ01"
REPO="Pebble"
PNAME="pebble"

nix_cmd=(
  nix
  --extra-experimental-features
  "nix-command flakes"
)

extract_hash() {
  local expr="$1"
  local output
  local hash

  if output="$("${nix_cmd[@]}" build --impure --no-link --expr "$expr" 2>&1)"; then
    echo "expected a fixed-output hash mismatch, but the build unexpectedly succeeded" >&2
    return 1
  fi

  hash="$(printf '%s\n' "$output" | sed -n 's/.*got:[[:space:]]*\(sha256-[A-Za-z0-9+/=]\+\).*/\1/p' | tail -n1)"
  if [[ -z "$hash" ]]; then
    printf '%s\n' "$output" >&2
    echo "failed to parse hash from nix output" >&2
    return 1
  fi

  printf '%s\n' "$hash"
}

if [[ $# -gt 1 ]]; then
  echo "usage: $0 [version-or-tag]" >&2
  exit 1
fi

if [[ $# -eq 1 ]]; then
  requested="$1"
  if [[ "$requested" == v* ]]; then
    new_tag="$requested"
  else
    new_tag="v$requested"
  fi
else
  new_tag="$(
    curl --fail --silent --show-error \
      "https://api.github.com/repos/$OWNER/$REPO/releases/latest" |
      jq --raw-output '.tag_name'
  )"
fi

if [[ -z "$new_tag" || "$new_tag" == "null" ]]; then
  echo "failed to determine the latest GitHub release tag" >&2
  exit 1
fi

if [[ "$new_tag" != v* ]]; then
  echo "unexpected release tag format: $new_tag (expected v<version>)" >&2
  exit 1
fi

new_version="${new_tag#v}"
current_version="$(sed -n 's/^[[:space:]]*version = "\([^"]*\)";/\1/p' "$PACKAGE_NIX" | head -n1)"

if [[ "$current_version" == "$new_version" ]]; then
  echo "$PNAME is already at $new_version"
  exit 0
fi

src_expr=$(cat <<EOF
let
  flake = builtins.getFlake "${REPO_ROOT}";
  pkgs = import flake.inputs.nixpkgs { system = builtins.currentSystem; };
in
pkgs.fetchFromGitHub {
  owner = "${OWNER}";
  repo = "${REPO}";
  rev = "${new_tag}";
  hash = pkgs.lib.fakeHash;
}
EOF
)

src_hash="$(extract_hash "$src_expr")"

pnpm_expr=$(cat <<EOF
let
  flake = builtins.getFlake "${REPO_ROOT}";
  pkgs = import flake.inputs.nixpkgs { system = builtins.currentSystem; };
  src = pkgs.fetchFromGitHub {
    owner = "${OWNER}";
    repo = "${REPO}";
    rev = "${new_tag}";
    hash = "${src_hash}";
  };
in
pkgs.fetchPnpmDeps {
  pname = "${PNAME}";
  version = "${new_version}";
  inherit src;
  fetcherVersion = 2;
  hash = pkgs.lib.fakeHash;
}
EOF
)

pnpm_hash="$(extract_hash "$pnpm_expr")"

sed -i \
  -e "s|^\(\s*version = \)\"[^\"]*\";|\1\"$new_version\";|" \
  -e "/src = fetchFromGitHub {/,/};/ s|^\(\s*hash = \)\"[^\"]*\";|\1\"$src_hash\";|" \
  -e "/pnpmDeps = fetchPnpmDeps {/,/};/ s|^\(\s*hash = \)\"[^\"]*\";|\1\"$pnpm_hash\";|" \
  "$PACKAGE_NIX"

echo "$PNAME -> $new_version"
echo "  src.hash      = $src_hash"
echo "  pnpmDeps.hash = $pnpm_hash"
