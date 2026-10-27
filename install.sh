#!/bin/sh
set -eu

REPO="ManiProjs/alacoup"
VERSION="${ALACOUP_VERSION:-latest}"
ROOT="${ALACOUP_HOME:-${HOME:?HOME is required}/.alacoup}"
BIN_DIR="$ROOT/bin"
TMP_DIR="$(mktemp -d)"
cleanup() { rm -rf "$TMP_DIR"; }
trap cleanup EXIT HUP INT TERM

case "$(uname -s):$(uname -m)" in
  Darwin:arm64|Darwin:aarch64) TARGET=aarch64-apple-darwin ;;
  Darwin:x86_64) TARGET=x86_64-apple-darwin ;;
  Linux:x86_64) TARGET=x86_64-unknown-linux-gnu ;;
  *) echo "Unsupported platform: $(uname -s) $(uname -m)" >&2; exit 1 ;;
esac

if [ "$VERSION" = latest ]; then
  URL="https://github.com/$REPO/releases/latest/download/alacoup-$TARGET.tar.gz"
else
  case "$VERSION" in
    v[0-9]*.[0-9]*.[0-9]*) ;;
    *) echo "ALACOUP_VERSION must be a release tag such as v0.1.0" >&2; exit 1 ;;
  esac
  URL="https://github.com/$REPO/releases/download/$VERSION/alacoup-$TARGET.tar.gz"
fi

curl --fail --location --silent --show-error --proto '=https' --tlsv1.2 "$URL" -o "$TMP_DIR/alacoup.tar.gz"
mkdir "$TMP_DIR/unpacked"
tar -xzf "$TMP_DIR/alacoup.tar.gz" -C "$TMP_DIR/unpacked" alacoup
test -f "$TMP_DIR/unpacked/alacoup"
chmod 755 "$TMP_DIR/unpacked/alacoup"
mkdir -p "$BIN_DIR"
install "$TMP_DIR/unpacked/alacoup" "$BIN_DIR/.alacoup-new"
mv -f "$BIN_DIR/.alacoup-new" "$BIN_DIR/alacoup"

echo "Installed AlacoUp into $BIN_DIR"
case ":${PATH}:" in *":$BIN_DIR:"*) ;; *) echo "Add this to your shell profile: export PATH=\"$BIN_DIR:\$PATH\"" ;; esac
"$BIN_DIR/alacoup" install
echo "AlacoUp and Alaco installation finished. Run: $BIN_DIR/alacoup doctor"
