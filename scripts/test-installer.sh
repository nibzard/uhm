#!/bin/sh
set -eu

root="$(mktemp -d)"
cleanup() { rm -rf "$root"; }
trap cleanup EXIT INT HUP TERM

mkdir -p "$root/bin" "$root/runtime" "$root/home" "$root/install"

cat > "$root/bin/curl" <<'EOF'
#!/bin/sh
dest=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = --output ]; then dest="$2"; shift 2; else shift; fi
done
case "$dest" in
  *SHA256SUMS) printf '%064d  uhm-v0.0.0-x86_64-unknown-linux-musl.tar.gz\n' 0 > "$dest" ;;
  *) : > "$dest" ;;
esac
EOF
cat > "$root/bin/sha256sum" <<'EOF'
#!/bin/sh
exit 0
EOF
cat > "$root/bin/tar" <<'EOF'
#!/bin/sh
owner_neutral=false
for argument in "$@"; do
  [ "$argument" = --no-same-owner ] && owner_neutral=true
done
$owner_neutral || exit 9
mkdir -p uhm-v0.0.0-x86_64-unknown-linux-musl
cat > uhm-v0.0.0-x86_64-unknown-linux-musl/uhm <<'INNER'
#!/bin/sh
printf 'uhm 0.0.0\n'
INNER
chmod 0755 uhm-v0.0.0-x86_64-unknown-linux-musl/uhm
EOF
chmod 0755 "$root/bin/curl" "$root/bin/sha256sum" "$root/bin/tar"

PATH="$root/bin:$PATH" \
HOME="$root/home" \
TMPDIR="$root/does-not-exist" \
XDG_RUNTIME_DIR="$root/runtime" \
UHM_VERSION=v0.0.0 \
UHM_TARGET=x86_64-unknown-linux-musl \
UHM_INSTALL_DIR="$root/install" \
sh docs/install.sh > "$root/output"

test -x "$root/install/uhm"
grep -F 'uhm 0.0.0' "$root/output" >/dev/null
test -z "$(find "$root/runtime" -mindepth 1 -maxdepth 1 -print -quit)"
printf 'installer portability test: ok\n'
