#!/bin/sh

set -eu

repo_owner="nibzard"
repo_name="uhm"
repo_slug="${repo_owner}/${repo_name}"
install_dir="${UHM_INSTALL_DIR:-${HOME}/.local/bin}"
default_version=""
requested_version="${UHM_VERSION:-}"
requested_target="${UHM_TARGET:-}"
tmp_root="${TMPDIR:-/tmp}"

say() {
  printf '%s\n' "$*"
}

fail() {
  printf 'uhm installer: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

download() {
  url="$1"
  dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl --fail --silent --show-error --location "$url" --output "$dest"
    return
  fi
  if command -v wget >/dev/null 2>&1; then
    wget -qO "$dest" "$url"
    return
  fi
  fail "need curl or wget to download release assets"
}

fetch_text() {
  url="$1"
  if command -v curl >/dev/null 2>&1; then
    curl --fail --silent --show-error --location "$url"
    return
  fi
  if command -v wget >/dev/null 2>&1; then
    wget -qO- "$url"
    return
  fi
  fail "need curl or wget to query release metadata"
}

resolve_version() {
  if [ -n "$requested_version" ]; then
    printf '%s\n' "$requested_version"
    return
  fi

  if [ -n "$default_version" ]; then
    printf '%s\n' "$default_version"
    return
  fi

  version="$(
    fetch_text "https://api.github.com/repos/${repo_slug}/releases/latest" \
      | sed -n 's/^[[:space:]]*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' \
      | head -n 1
  )"
  [ -n "$version" ] || fail "could not resolve the latest release tag; set UHM_VERSION=vX.Y.Z"
  printf '%s\n' "$version"
}

resolve_target() {
  if [ -n "$requested_target" ]; then
    printf '%s\n' "$requested_target"
    return
  fi

  kernel="$(uname -s)"
  machine="$(uname -m)"

  case "$kernel" in
    Linux) os="unknown-linux-musl" ;;
    Darwin) os="apple-darwin" ;;
    *) fail "unsupported operating system: $kernel" ;;
  esac

  case "$machine" in
    x86_64|amd64) arch="x86_64" ;;
    aarch64|arm64) arch="aarch64" ;;
    *) fail "unsupported architecture: $machine" ;;
  esac

  printf '%s-%s\n' "$arch" "$os"
}

verify_sha256() {
  manifest="$1"
  archive="$2"
  archive_name="$(basename "$archive")"
  checkfile="${archive}.sha256"

  grep -F "  ${archive_name}" "$manifest" > "$checkfile" \
    || fail "checksum entry missing for ${archive_name}"

  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$(dirname "$archive")" && sha256sum -c "$(basename "$checkfile")") >/dev/null
    return
  fi
  if command -v shasum >/dev/null 2>&1; then
    (cd "$(dirname "$archive")" && shasum -a 256 -c "$(basename "$checkfile")") >/dev/null
    return
  fi
  if command -v openssl >/dev/null 2>&1; then
    expected="$(awk '{print $1}' "$checkfile")"
    actual="$(openssl dgst -sha256 "$archive" | sed 's/^.*= //')"
    [ "$expected" = "$actual" ] || fail "checksum mismatch for ${archive_name}"
    return
  fi

  fail "need sha256sum, shasum, or openssl to verify downloads"
}

install_binary() {
  source_binary="$1"
  destination_dir="$2"
  destination_path="${destination_dir}/uhm"

  mkdir -p "$destination_dir"
  if command -v install >/dev/null 2>&1; then
    install -m 0755 "$source_binary" "$destination_path"
  else
    cp "$source_binary" "$destination_path"
    chmod 0755 "$destination_path"
  fi
}

need_cmd tar
need_cmd mktemp
need_cmd grep
need_cmd sed
need_cmd awk
need_cmd uname

version="$(resolve_version)"
target="$(resolve_target)"
archive="uhm-${version}-${target}.tar.gz"
release_base="https://github.com/${repo_slug}/releases/download/${version}"
tmp_dir="$(mktemp -d "${tmp_root%/}/uhm-install.XXXXXX")"

cleanup() {
  rm -rf "$tmp_dir"
}

trap cleanup EXIT INT HUP TERM

say "uhm installer: downloading ${archive}"
download "${release_base}/${archive}" "${tmp_dir}/${archive}"
download "${release_base}/SHA256SUMS" "${tmp_dir}/SHA256SUMS"

say "uhm installer: verifying checksum"
verify_sha256 "${tmp_dir}/SHA256SUMS" "${tmp_dir}/${archive}"

say "uhm installer: extracting ${archive}"
(cd "$tmp_dir" && tar -xzf "$archive")

source_binary="${tmp_dir}/uhm-${version}-${target}/uhm"
[ -x "$source_binary" ] || fail "release archive did not contain an executable uhm binary"

say "uhm installer: installing to ${install_dir}"
install_binary "$source_binary" "$install_dir"

destination_path="${install_dir}/uhm"
say "uhm installer: installed ${destination_path}"
"$destination_path" --version

case ":${PATH:-}:" in
  *:"${install_dir}":*) ;;
  *)
    say ""
    say "Add ${install_dir} to PATH if it is not already there:"
    say "  export PATH=\"${install_dir}:\$PATH\""
    ;;
esac

say ""
say "Next:"
say "  uhm doctor"
say "  uhm doctor network"
