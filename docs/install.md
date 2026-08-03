# Install

`uhm` runs on Linux and macOS. It is a single binary — no runtime, no daemon, no background services. Linux builds are statically linked (musl); macOS builds link the system shared libraries. Native Windows is not supported.

You need an [OpenAI API key](https://platform.openai.com/api-keys); `uhm` calls the Responses API with `store: false`. Set it before your first invocation:

```sh
export OPENAI_API_KEY="sk-..."
```

The key is read from the environment first, then from a private `0600` secrets file whose path `uhm doctor` prints. There is no account, login, or cloud history.

## Option 1 — prebuilt binary (recommended)

Each release publishes four native archives with SHA-256 checksums (`SHA256SUMS`) and GitHub build-provenance attestations:

| Archive | Target |
|---|---|
| `uhm-v0.3.0-x86_64-unknown-linux-musl.tar.gz` | Linux, Intel/AMD 64-bit |
| `uhm-v0.3.0-aarch64-unknown-linux-musl.tar.gz` | Linux, ARM 64-bit |
| `uhm-v0.3.0-x86_64-apple-darwin.tar.gz` | macOS, Intel |
| `uhm-v0.3.0-aarch64-apple-darwin.tar.gz` | macOS, Apple Silicon |

Download from the [v0.3.0 release page](https://github.com/nibzard/uhm/releases/tag/v0.3.0), then verify and extract:

```sh
# choose the archive that matches your machine
archive=uhm-v0.3.0-x86_64-unknown-linux-musl.tar.gz

curl -LO "https://github.com/nibzard/uhm/releases/download/v0.3.0/${archive}"
curl -LO "https://github.com/nibzard/uhm/releases/download/v0.3.0/SHA256SUMS"
grep -F "  ${archive}" SHA256SUMS > "${archive}.sha256"
if [ "$(uname -s)" = Darwin ]; then
  shasum -a 256 -c "${archive}.sha256"
else
  sha256sum -c "${archive}.sha256"
fi
tar -xzf "${archive}"
mkdir -p "${HOME}/.local/bin"
install -m 0755 uhm-v0.3.0-*/uhm "${HOME}/.local/bin/uhm"
```

The checksum command prints `<archive>: OK` when the download matches the manifest.

### macOS quarantine

macOS archives are not Developer ID signed or notarized in this release, so the first launch may be blocked by Gatekeeper. Approve it once in *System Settings → Privacy & Security*, or clear the quarantine attribute from the file you just verified:

```sh
xattr -d com.apple.quarantine "${HOME}/.local/bin/uhm"
```

### PATH

Ensure the install directory is on your `PATH`:

```sh
# bash or zsh
echo 'export PATH="${HOME}/.local/bin:${PATH}"' >> "${HOME}/.${SHELL##*/}rc"

# fish (run from fish)
fish_add_path "${HOME}/.local/bin"
```

## Option 2 — build from source

With Rust 1.82 or newer:

```sh
cargo install --locked --git https://github.com/nibzard/uhm --tag v0.3.0 uhm-cli
```

This builds the `uhm-cli` crate and installs the `uhm` binary into `~/.cargo/bin`, which `cargo` already manages on your `PATH`. `--locked` uses the exact pinned dependencies from `Cargo.lock`.

The crate passes `cargo publish --dry-run`, but publishing to crates.io is deferred for v0.3.0; GitHub binaries and `cargo install --git` are the supported channels.

## Verify

```sh
uhm --version        # uhm 0.3.0
uhm doctor           # local configuration and terminal checks
uhm doctor network   # confirm OpenAI is reachable and the key works
```

## Next steps

- [Quickstart](getting-started.md) — first result in under five minutes
- [CLI reference](cli-reference.md) — every command and flag
- [Configuration](configuration.md) — `config.yaml`, model precedence, and aliases
