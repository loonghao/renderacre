#!/usr/bin/env bash
set -euo pipefail

repo="${RENDERACRE_REPO:-loonghao/renderacre}"
version="${RENDERACRE_VERSION:-latest}"
install_dir="${RENDERACRE_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s)" in
  Linux) os="linux" ;;
  Darwin) os="macos" ;;
  *) echo "unsupported OS: $(uname -s)" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64|amd64) arch="x86_64" ;;
  arm64|aarch64) arch="arm64" ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

if [[ "$version" == "latest" ]]; then
  if command -v curl >/dev/null 2>&1; then
    version="$(curl -fsSL "https://api.github.com/repos/${repo}/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)"
  elif command -v wget >/dev/null 2>&1; then
    version="$(wget -qO- "https://api.github.com/repos/${repo}/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)"
  else
    echo "curl or wget is required" >&2
    exit 1
  fi
  if [[ -z "$version" ]]; then
    echo "could not resolve latest renderacre release" >&2
    exit 1
  fi
fi

asset="renderacre-${version}-${os}-${arch}.tar.gz"
url="https://github.com/${repo}/releases/download/${version}/${asset}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$install_dir"
echo "Downloading $url"
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$url" -o "$tmp/$asset"
elif command -v wget >/dev/null 2>&1; then
  wget -q "$url" -O "$tmp/$asset"
else
  echo "curl or wget is required" >&2
  exit 1
fi

tar -xzf "$tmp/$asset" -C "$tmp"
bundle_dir="$(find "$tmp" -maxdepth 1 -type d -name 'renderacre-*' | head -n 1)"
install -m 0755 "$bundle_dir/renderacre-controller" "$install_dir/renderacre-controller"
install -m 0755 "$bundle_dir/renderacre-worker" "$install_dir/renderacre-worker"

echo "Installed renderacre-controller and renderacre-worker to $install_dir"
echo "Make sure $install_dir is on PATH."
