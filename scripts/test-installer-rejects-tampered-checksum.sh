#!/bin/sh
# Negative complement to test-installer.sh. The portability harness stubs
# `sha256sum` to `exit 0`, so a genuine checksum mismatch is never exercised.
# This script uses the REAL `sha256sum` (it is not placed on PATH) and asserts
# the installer refuses to install an archive whose recorded digest does not
# match. With release signing configured, SHA256SUMS itself is authenticated by
# a detached signature before this check runs; this guards the installer's own
# archive-versus-manifest comparison regardless.
set -eu

root="$(mktemp -d)"
cleanup() { rm -rf "$root"; }
trap cleanup EXIT INT HUP TERM

mkdir -p "$root/bin" "$root/home" "$root/install"

# curl: hand back a manifest whose recorded hash is all zeros (it can never
# match the real digest of the empty archive) and an empty archive.
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

# tar: simulate a successful extraction that yields an executable uhm. install.sh
# runs verify_sha256 BEFORE extraction, so this stub is reached only if the
# checksum gate is bypassed — which is exactly the regression to catch. Making
# the stub produce a real binary means verify_sha256 is the SOLE gate standing
# between this input and a completed install; if a future change swallows its
# non-zero exit, the install would succeed and this test would fail.
cat > "$root/bin/tar" <<'EOF'
#!/bin/sh
mkdir -p uhm-v0.0.0-x86_64-unknown-linux-musl
cat > uhm-v0.0.0-x86_64-unknown-linux-musl/uhm <<'INNER'
#!/bin/sh
printf 'uhm 0.0.0\n'
INNER
chmod 0755 uhm-v0.0.0-x86_64-unknown-linux-musl/uhm
EOF
chmod 0755 "$root/bin/curl" "$root/bin/tar"

# Do NOT stub sha256sum: let the platform's real one run so a mismatch is
# detected. Skip gracefully where it is unavailable.
if ! command -v sha256sum >/dev/null 2>&1; then
  printf 'skipping negative-checksum test: sha256sum unavailable\n'
  exit 0
fi

if PATH="$root/bin:$PATH" \
   HOME="$root/home" \
   TMPDIR="$root" \
   UHM_VERSION=v0.0.0 \
   UHM_TARGET=x86_64-unknown-linux-musl \
   UHM_INSTALL_DIR="$root/install" \
   sh docs/install.sh >"$root/output" 2>"$root/err"; then
  printf 'FAIL: installer accepted a tampered checksum\n' >&2
  cat "$root/output" >&2
  exit 1
fi

if [ -e "$root/install/uhm" ]; then
  printf 'FAIL: installer installed a binary despite the checksum mismatch\n' >&2
  exit 1
fi

printf 'installer rejects tampered checksum: ok\n'
