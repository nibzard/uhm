#!/usr/bin/env bash
# The commands shown in the public demo. The recorder supplies an isolated HOME,
# repository, config, and an absolute UHM_DEMO_BIN outside the captured command text.
set -euo pipefail

: "${UHM_DEMO_BIN:?the recorder must set UHM_DEMO_BIN}"

prompt=$'\033[1;36m❯\033[0m '

type_line() {
  local line=$1
  local i
  printf '%s' "$prompt"
  for ((i = 0; i < ${#line}; i++)); do
    printf '%s' "${line:i:1}"
    sleep 0.065
  done
  printf '\n'
}

beat() {
  sleep 1
}

type_line 'uhm list the three biggest files'
"$UHM_DEMO_BIN" list the three biggest files
beat

type_line 'git diff | uhm ask write a one-line commit message'
git diff | "$UHM_DEMO_BIN" ask write a one-line commit message
beat

type_line 'uhm count the words, paragraphs, and headings in the markdown files in docs'
"$UHM_DEMO_BIN" count the words, paragraphs, and headings in the markdown files in docs
beat

type_line 'uhm explain what git log -p does'
"$UHM_DEMO_BIN" explain what git log -p does
beat

type_line 'uhm run --dry-run concatenate the markdown files'
"$UHM_DEMO_BIN" run --dry-run concatenate the markdown files
printf '\n'
beat

type_line 'uhm --force remove build artifacts'
"$UHM_DEMO_BIN" --force remove build artifacts
