#!/usr/bin/env bash
# Record the real OpenAI-backed walkthrough, reject private material, and render
# the committed cast with pinned tools. Use --render-only to avoid API access.
set -euo pipefail

ASCIINEMA_VERSION=3.2.0
SVG_TERM_VERSION=2.1.1
AGG_VERSION=1.9.0
ASSET_BUDGET_BYTES=$((6 * 1024 * 1024))

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cast_path="$repo_root/docs/demo/uhm-demo.cast"
svg_path="$repo_root/docs/demo/uhm-demo.svg"
gif_path="$repo_root/docs/demo/uhm-demo.gif"
driver="$repo_root/scripts/demo/demo-script.sh"
mode=${1:-record}

die() {
  printf 'record-demo: %s\n' "$*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

if [[ $mode != record && $mode != --render-only ]]; then
  die 'usage: scripts/record-demo.sh [--render-only]'
fi

need npx
need node
need agg

[[ $(agg --version) == "agg $AGG_VERSION" ]] || die "agg $AGG_VERSION is required"

stage=$(mktemp -d "${TMPDIR:-/tmp}/uhm-demo.XXXXXX")
cleanup() {
  rm -rf -- "$stage"
}
trap cleanup EXIT HUP INT TERM

render() {
  local source_cast=$1
  local staged_svg="$stage/uhm-demo.svg"
  local staged_gif="$stage/uhm-demo.gif"

  npx --yes "svg-term-cli@$SVG_TERM_VERSION" \
    --in "$source_cast" --out "$staged_svg" --window --width 80 --height 24 \
    --padding-x 12 --padding-y 10 --no-cursor
  agg --quiet --cols 80 --rows 24 --font-size 13 --theme nord \
    --speed 1.35 --idle-time-limit 1.2 --fps-cap 12 \
    "$source_cast" "$staged_gif"

  local total_bytes
  total_bytes=$(wc -c < "$source_cast")
  total_bytes=$((total_bytes + $(wc -c < "$staged_svg") + $(wc -c < "$staged_gif")))
  ((total_bytes <= ASSET_BUDGET_BYTES)) || \
    die "demo assets use $total_bytes bytes; budget is $ASSET_BUDGET_BYTES"

  cp "$staged_svg" "$svg_path"
  cp "$staged_gif" "$gif_path"
  printf 'Rendered SVG and GIF from %s (%s bytes total).\n' "$source_cast" "$total_bytes"
}

if [[ $mode == --render-only ]]; then
  [[ -f $cast_path ]] || die "missing canonical cast: $cast_path"
  render "$cast_path"
  exit 0
fi

need asciinema
need cargo
need git
[[ $(asciinema --version) == "asciinema $ASCIINEMA_VERSION" ]] || \
  die "asciinema $ASCIINEMA_VERSION is required"
[[ -n ${OPENAI_API_KEY:-} ]] || die 'OPENAI_API_KEY must be set before recording'

real_home=${HOME:-}
real_hostname=$(hostname 2>/dev/null || true)
key_fragment=${OPENAI_API_KEY:0:12}

printf 'Building uhm...\n'
(cd "$repo_root" && cargo build --release --locked)

demo_home="$stage/home"
workspace="$stage/workspace"
mkdir -p "$demo_home/config/uhm" "$demo_home/data/uhm" "$demo_home/cache" \
  "$workspace/docs" "$workspace/build"
chmod 700 "$demo_home" "$demo_home/config" "$demo_home/data" "$demo_home/cache"

printf '%s\n' \
  '# Field notes' \
  '' \
  'The release is ready for a final pass.' > "$workspace/docs/notes.md"
printf '%s\n' \
  '# Checklist' \
  '' \
  '- Verify archives' \
  '- Publish notes' > "$workspace/docs/checklist.md"
dd if=/dev/zero of="$workspace/sample-data.bin" bs=1024 count=96 status=none
dd if=/dev/zero of="$workspace/build/cache.bin" bs=1024 count=48 status=none
printf 'small fixture\n' > "$workspace/tiny.txt"

(
  cd "$workspace"
  git init -q
  git config user.name 'Demo User'
  git config user.email 'demo@example.invalid'
  git add .
  git commit -qm 'seed demo workspace'
  printf '\nThe checksum list is attached to the release.\n' >> docs/notes.md
)

printf '%s\n' \
  'max_completion_tokens: 1024' \
  'cache_enabled: false' \
  'history:' \
  '  enabled: false' \
  'telemetry:' \
  '  enabled: false' \
  'aliases:' \
  "  'concatenate the markdown files': 'cat docs/*.md > combined.md'" \
  "  'remove build artifacts': 'rm -rf -- build && echo Build artifacts removed.'" > "$demo_home/config/uhm/config.yaml"
printf '3\n' > "$demo_home/data/uhm/notice-revision"

staged_cast="$stage/uhm-demo.cast"
printf 'Recording a real session in an isolated 80x24 workspace...\n'
set +e
(
  cd "$workspace"
  export HOME="$demo_home"
  export XDG_CONFIG_HOME="$demo_home/config"
  export XDG_DATA_HOME="$demo_home/data"
  export XDG_CACHE_HOME="$demo_home/cache"
  export TERM=xterm-256color
  export SHELL=/bin/bash
  export NO_COLOR=
  export CLICOLOR_FORCE=1
  export UHM_DEMO_BIN="$repo_root/target/release/uhm"
  asciinema record --quiet --headless --return --overwrite \
    --output-format asciicast-v2 --window-size 80x24 --idle-time-limit 1.2 \
    --title 'uhm — say what you need; get the result' \
    --command "bash '$driver'" "$staged_cast"
)
record_status=$?
set -e
if [[ $record_status -ne 0 ]]; then
  failed_cast="$repo_root/target/uhm-demo-failed.cast"
  cp "$staged_cast" "$failed_cast"
  die "recorded session exited $record_status; inspect $failed_cast"
fi

# v2 players do not require these recorder-specific fields. Removing them keeps
# checkout paths out of published metadata and avoids a new timestamp-only diff.
node - "$staged_cast" "$stage/normalized.cast" <<'NODE'
const fs = require('fs');
const [source, destination] = process.argv.slice(2);
const lines = fs.readFileSync(source, 'utf8').trimEnd().split('\n');
const header = JSON.parse(lines[0]);
delete header.command;
delete header.timestamp;
lines[0] = JSON.stringify(header);
fs.writeFileSync(destination, `${lines.join('\n')}\n`);
NODE
mv "$stage/normalized.cast" "$staged_cast"

scan_for() {
  local label=$1
  local needle=$2
  [[ -z $needle ]] && return 0
  if LC_ALL=C grep -aFq -- "$needle" "$staged_cast"; then
    die "privacy check failed: cast contains $label"
  fi
}

scan_for 'an API-key prefix' 'sk-'
scan_for 'the configured key fragment' "$key_fragment"
scan_for 'the recording machine home path' "$real_home"
scan_for 'the recording machine hostname' "$real_hostname"
scan_for 'the repository path' "$repo_root"

cp "$staged_cast" "$cast_path"
render "$cast_path"
printf 'Privacy checks passed. Canonical cast: %s\n' "$cast_path"
