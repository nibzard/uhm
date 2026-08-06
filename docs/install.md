<!-- diataxis: how-to -->

# Install

`uhm` runs on Linux and macOS. It is a single binary — no runtime, no daemon, no background services. Linux builds are statically linked (musl); macOS builds link the system shared libraries. Native Windows is not supported.

OpenAI is the default provider. You need an [OpenAI API key](https://platform.openai.com/api-keys); `uhm` calls the Responses API with `store: false`. Set it before your first invocation:

```sh
export OPENAI_API_KEY="sk-..."
```

The key is read from the environment first, then from a private `0600` secrets file whose path `uhm doctor` prints. There is no account, login, or cloud history.

To use Cerebras explicitly instead, set its key and select both provider and model:

```sh
export CEREBRAS_API_KEY="csk-..."
uhm --provider cerebras --model gpt-oss-120b doctor network
```

This verifies endpoint compatibility and credentials; it does not claim that the pair is qualified for automatic evidence-based selection. See [Configuration](configuration.md) for persistent selection and optional fallback.

## Option 1 — one-line installer (recommended)

```sh
curl -fsSL https://nibzard.github.io/uhm/install.sh | sh
```

This installer follows the release-archive path already used by `uhm`: it detects your platform, downloads the matching archive and `SHA256SUMS` from GitHub Releases, verifies the checksum, and installs `uhm` into `~/.local/bin` by default.

It intentionally does not edit your shell startup files. If `~/.local/bin` is not on `PATH`, add it yourself after install.

The GitHub Pages URL above tracks the current `main` branch and installs the latest release by default. If you want an immutable script tied to one release, use that release's asset instead:

```sh
curl -fsSL https://github.com/nibzard/uhm/releases/download/v0.6.1/install.sh | sh
```

That release-hosted script is pinned to `v0.6.1` unless you override it with `UHM_VERSION=...`.

Useful overrides:

```sh
UHM_VERSION=v0.6.1 curl -fsSL https://nibzard.github.io/uhm/install.sh | sh
UHM_INSTALL_DIR="$HOME/bin" curl -fsSL https://nibzard.github.io/uhm/install.sh | sh
UHM_TARGET=aarch64-unknown-linux-musl curl -fsSL https://nibzard.github.io/uhm/install.sh | sh
```

Inspect the script before running it if you prefer:

```sh
curl -fsSL https://nibzard.github.io/uhm/install.sh | less
```

## Option 2 — prebuilt binary (manual)

Each release publishes four native archives with SHA-256 checksums (`SHA256SUMS`) and GitHub build-provenance attestations:

| Archive | Target |
|---|---|
| `uhm-v0.6.1-x86_64-unknown-linux-musl.tar.gz` | Linux, Intel/AMD 64-bit |
| `uhm-v0.6.1-aarch64-unknown-linux-musl.tar.gz` | Linux, ARM 64-bit |
| `uhm-v0.6.1-x86_64-apple-darwin.tar.gz` | macOS, Intel |
| `uhm-v0.6.1-aarch64-apple-darwin.tar.gz` | macOS, Apple Silicon |

Download from the [v0.6.1 release page](https://github.com/nibzard/uhm/releases/tag/v0.6.1), then verify and extract:

```sh
# choose the archive that matches your machine
archive=uhm-v0.6.1-x86_64-unknown-linux-musl.tar.gz

curl -LO "https://github.com/nibzard/uhm/releases/download/v0.6.1/${archive}"
curl -LO "https://github.com/nibzard/uhm/releases/download/v0.6.1/SHA256SUMS"
grep -F "  ${archive}" SHA256SUMS > "${archive}.sha256"
if [ "$(uname -s)" = Darwin ]; then
  shasum -a 256 -c "${archive}.sha256"
else
  sha256sum -c "${archive}.sha256"
fi
tar --no-same-owner -xzf "${archive}"
mkdir -p "${HOME}/.local/bin"
install -m 0755 uhm-v0.6.1-*/uhm "${HOME}/.local/bin/uhm"
```

The checksum command prints `<archive>: OK` when the download matches the manifest.

The installer uses an existing writable `TMPDIR`, then `XDG_RUNTIME_DIR`, then `/tmp`, and finally a private directory beneath the user cache. If none is available it exits with instructions instead of assuming `/tmp` exists.

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

## Option 3 — build from source

With Rust 1.89 or newer:

```sh
cargo install --locked --git https://github.com/nibzard/uhm --tag v0.6.1 uhm-cli
```

This builds the `uhm-cli` crate and installs the `uhm` binary into `~/.cargo/bin`, which `cargo` already manages on your `PATH`. `--locked` uses the exact pinned dependencies from `Cargo.lock`.

The crate passes `cargo publish --dry-run`, but publishing to crates.io is deferred for v0.6.1; GitHub binaries and `cargo install --git` are the supported channels.

## Verify

```sh
uhm --version        # uhm 0.6.1
uhm doctor           # local configuration and terminal checks
uhm doctor network   # confirm the selected provider is reachable and its key works
```

## Next steps

- [Quickstart](getting-started.md) — first result in under five minutes
- [CLI reference](cli-reference.md) — every command and flag
- [Configuration](configuration.md) — providers, credentials, selection, and aliases
