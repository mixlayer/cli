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
cp target/release/mxl target/release/mxl-mcp README.md LICENSE CHANGELOG.md "$t/pkg/$name/"
(cd "$t/pkg" && tar czf "$t/rel/$name.tar.gz" "$name")
if command -v sha256sum >/dev/null 2>&1; then
	(cd "$t/rel" && sha256sum mixlayer-* >SHA256SUMS)
else
	(cd "$t/rel" && shasum -a 256 mixlayer-* >SHA256SUMS)
fi

install() { MIXLAYER_RELEASE_URL="file://$t/rel" MIXLAYER_INSTALL_DIR="$t/$1" sh install.sh >"$t/$1.log" 2>&1; }

install bin || { cat "$t/bin.log"; echo "FAIL: install"; exit 1; }
"$t/bin/mxl" version >/dev/null
[ -x "$t/bin/mxl-mcp" ] || { echo "FAIL: mxl-mcp not installed"; exit 1; }
"$t/bin/mxl-mcp" </dev/null >/dev/null || { echo "FAIL: mxl-mcp did not exit cleanly on EOF"; exit 1; }
install bin || { echo "FAIL: reinstall"; exit 1; }

mkdir -p "$t/compat" "$t/foreign" "$t/no-compat"
printf '#!/bin/sh\necho Mixlayer 0.2.0\n' >"$t/compat/mixlayer"
printf '#!/bin/sh\nexit 0\n' >"$t/compat/mixlayer-mcp"
chmod +x "$t/compat/mixlayer" "$t/compat/mixlayer-mcp"
install compat || { cat "$t/compat.log"; exit 1; }
[ "$(readlink "$t/compat/mixlayer")" = mxl ] || { echo "FAIL: legacy CLI not linked"; exit 1; }
[ "$(readlink "$t/compat/mixlayer-mcp")" = mxl-mcp ] || { echo "FAIL: legacy MCP not linked"; exit 1; }

printf '#!/bin/sh\necho other-tool\n' >"$t/foreign/mixlayer"
chmod +x "$t/foreign/mixlayer"
install foreign || { cat "$t/foreign.log"; exit 1; }
[ ! -L "$t/foreign/mixlayer" ] || { echo "FAIL: foreign binary replaced"; exit 1; }

cp "$t/compat/mxl" "$t/no-compat/mixlayer"
cp "$t/compat/mxl-mcp" "$t/no-compat/mixlayer-mcp"
MIXLAYER_NO_COMPAT=1 install no-compat || { cat "$t/no-compat.log"; exit 1; }
[ ! -L "$t/no-compat/mixlayer" ] || { echo "FAIL: compatibility skip ignored"; exit 1; }

cp "$t/rel/$name.tar.gz" "$t/good"
printf x >>"$t/rel/$name.tar.gz"
if install tampered; then echo "FAIL: tampered archive installed"; exit 1; fi
grep -q "checksum mismatch" "$t/tampered.log" || { cat "$t/tampered.log"; exit 1; }
[ ! -e "$t/tampered/mxl" ] || { echo "FAIL: tampered binary written"; exit 1; }
cp "$t/good" "$t/rel/$name.tar.gz"

: >"$t/rel/SHA256SUMS"
if install unlisted; then echo "FAIL: unlisted archive installed"; exit 1; fi
grep -q "not listed" "$t/unlisted.log" || { cat "$t/unlisted.log"; exit 1; }

echo "install.sh: ok (install, reinstall, compatibility, foreign binary, skip, tampered archive refused, unlisted archive refused)"
