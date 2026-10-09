#!/bin/sh
# Install the Mixlayer CLI (mixlayer and mixlayer-mcp) from GitHub Releases.
#
#   curl -fsSL https://github.com/mixlayer/cli/releases/latest/download/install.sh | sh
#
# Environment:
#   MIXLAYER_INSTALL_DIR  where to put the binaries (default ~/.local/bin)
#   MIXLAYER_VERSION      a release tag such as v0.1.0 (default: latest)
#   MIXLAYER_RELEASE_URL  base URL of the release assets (for mirrors and tests)
set -eu

repo="https://github.com/mixlayer/cli"
dir="${MIXLAYER_INSTALL_DIR:-$HOME/.local/bin}"

fail() {
	echo "mixlayer install: $*" >&2
	exit 1
}

case "$(uname -s)" in
Darwin) os=apple-darwin ;;
Linux) os=unknown-linux-musl ;;
*) fail "unsupported OS $(uname -s); download a build from $repo/releases" ;;
esac
case "$(uname -m)" in
x86_64 | amd64) arch=x86_64 ;;
arm64 | aarch64) arch=aarch64 ;;
*) fail "unsupported CPU $(uname -m); download a build from $repo/releases" ;;
esac
target="$arch-$os"

if [ -n "${MIXLAYER_RELEASE_URL:-}" ]; then
	base="$MIXLAYER_RELEASE_URL"
elif [ -n "${MIXLAYER_VERSION:-}" ]; then
	base="$repo/releases/download/$MIXLAYER_VERSION"
else
	base="$repo/releases/latest/download"
fi

fetch() {
	if command -v curl >/dev/null 2>&1; then
		curl -fsSL "$1" -o "$2"
	elif command -v wget >/dev/null 2>&1; then
		wget -q "$1" -O "$2"
	else
		fail "needs curl or wget"
	fi
}

sha256() {
	if command -v sha256sum >/dev/null 2>&1; then
		sha256sum "$1" | cut -d' ' -f1
	else
		shasum -a 256 "$1" | cut -d' ' -f1
	fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

archive="mixlayer-$target.tar.gz"
echo "Downloading $archive"
fetch "$base/$archive" "$tmp/$archive" || fail "could not download $base/$archive"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || fail "could not download $base/SHA256SUMS"

want="$(awk -v f="$archive" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS")"
[ -n "$want" ] || fail "$archive is not listed in SHA256SUMS"
got="$(sha256 "$tmp/$archive")"
[ "$want" = "$got" ] || fail "checksum mismatch for $archive"

tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$dir"
for bin in mixlayer mixlayer-mcp; do
	src="$tmp/mixlayer-$target/$bin"
	[ -f "$src" ] || fail "$bin missing from $archive"
	# Replace atomically so a running copy is not corrupted.
	cp "$src" "$dir/.$bin.new"
	chmod 755 "$dir/.$bin.new"
	mv -f "$dir/.$bin.new" "$dir/$bin"
done

echo "Installed $(MIXLAYER_NO_AGENT_DETECT=1 "$dir/mixlayer" version) to $dir"
case ":$PATH:" in
*":$dir:"*) ;;
*) echo "Add $dir to your PATH, for example: export PATH=\"$dir:\$PATH\"" ;;
esac
echo "Next: create a key at https://console.mixlayer.com/app/api-keys, then run 'mixlayer doctor'."
