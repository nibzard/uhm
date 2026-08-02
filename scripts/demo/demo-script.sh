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

type_line 'uhm --context minimal -- find the ten biggest files in this directory'
"$UHM_DEMO_BIN" --context minimal -- find the ten biggest files in this directory
beat

type_line 'git diff | uhm ask --context minimal -- write a concise summary'
git diff | "$UHM_DEMO_BIN" ask --context minimal -- write a concise summary
beat

type_line 'uhm explain --context minimal -- git log -p'
"$UHM_DEMO_BIN" explain --context minimal -- git log -p
beat

type_line 'uhm run --dry-run --context minimal -- concatenate the markdown files'
"$UHM_DEMO_BIN" run --dry-run --context minimal -- concatenate the markdown files
printf '\n'
beat

type_line 'uhm run --context minimal -- remove build artifacts'
printf -v quoted_bin '%q' "$UHM_DEMO_BIN"
set +e
if [[ $(uname -s) == Darwin ]]; then
  (sleep 1; printf 'q\n') | script -q /dev/null "$UHM_DEMO_BIN" run --context minimal -- remove build artifacts
else
  (sleep 1; printf 'q\n') | script -q -e -c "$quoted_bin run --context minimal -- remove build artifacts" /dev/null
fi
cancel_status=$?
set -e
if [[ $cancel_status -ne 0 && $cancel_status -ne 11 ]]; then
  exit "$cancel_status"
fi

printf '\n\033[1;36mCancelled. Nothing was removed.\033[0m\n'
