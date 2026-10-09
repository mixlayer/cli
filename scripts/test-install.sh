#!/bin/sh
# Test install.sh against a local release packaged the way release.yml does:
# a good install, a reinstall, a tampered archive and a missing checksum.
# Run after `cargo build --release`.
set -eu
cd "$(dirname "$0")/.."
case "$(uname -s)" in
Darwin) os=apple-darwin ;;
Linux) os=unknown-linux-musl ;;
*) echo "skip: unsupported OS" && exit 0 ;;
esac
case "$(uname -m)" in
x86_64 | amd64) arch=x86_64 ;;
*) arch=aarch64 ;;
esac
name="mixlayer-$arch-$os"
t="$(mktemp -d)"
trap 'rm -rf "$t"' EXIT
mkdir -p "$t/pkg/$name" "$t/rel"
cp target/release/mixlayer target/release/mixlayer-mcp README.md LICENSE CHANGELOG.md "$t/pkg/$name/"
(cd "$t/pkg" && tar czf "$t/rel/$name.tar.gz" "$name")
if command -v sha256sum >/dev/null 2>&1; then
	(cd "$t/rel" && sha256sum mixlayer-* >SHA256SUMS)
else
	(cd "$t/rel" && shasum -a 256 mixlayer-* >SHA256SUMS)
fi

install() { MIXLAYER_RELEASE_URL="file://$t/rel" MIXLAYER_INSTALL_DIR="$t/$1" sh install.sh >"$t/$1.log" 2>&1; }

install bin || { cat "$t/bin.log"; echo "FAIL: install"; exit 1; }
"$t/bin/mixlayer" version >/dev/null
[ -x "$t/bin/mixlayer-mcp" ] || { echo "FAIL: mixlayer-mcp not installed"; exit 1; }
"$t/bin/mixlayer-mcp" </dev/null >/dev/null || { echo "FAIL: mixlayer-mcp did not exit cleanly on EOF"; exit 1; }
install bin || { echo "FAIL: reinstall"; exit 1; }

cp "$t/rel/$name.tar.gz" "$t/good"
printf x >>"$t/rel/$name.tar.gz"
if install tampered; then echo "FAIL: tampered archive installed"; exit 1; fi
grep -q "checksum mismatch" "$t/tampered.log" || { cat "$t/tampered.log"; exit 1; }
[ ! -e "$t/tampered/mixlayer" ] || { echo "FAIL: tampered binary written"; exit 1; }
cp "$t/good" "$t/rel/$name.tar.gz"

: >"$t/rel/SHA256SUMS"
if install unlisted; then echo "FAIL: unlisted archive installed"; exit 1; fi
grep -q "not listed" "$t/unlisted.log" || { cat "$t/unlisted.log"; exit 1; }

echo "install.sh: ok (install, reinstall, tampered archive refused, unlisted archive refused)"
