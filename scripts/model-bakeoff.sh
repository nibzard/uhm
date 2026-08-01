#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${OPENAI_API_KEY:-}" ]]; then
  echo "OPENAI_API_KEY is required" >&2
  exit 2
fi
if ! command -v jq >/dev/null; then
  echo "jq is required" >&2
  exit 2
fi
if ! command -v python3 >/dev/null; then
  echo "python3 is required for portable millisecond timing" >&2
  exit 2
fi

now_ms() { python3 -c 'import time; print(time.time_ns() // 1000000)'; }

candidates=("${@:-gpt-5.6-luna gpt-5.6-terra gpt-5.6-sol}")
corpus="tests/fixtures/result-first-eval.json"
cargo build --quiet

for model in ${candidates[*]}; do
  while IFS=$'\t' read -r id mode prompt expected; do
    start="$(now_ms)"
    set +e
    output="$(target/debug/uhm "$mode" --context minimal --fresh --dry-run --json --model "$model" -- "$prompt" </dev/null 2>/dev/null)"
    status=$?
    set -e
    elapsed=$(( $(now_ms) - start ))
    actual="$(jq -r '.outcome // "invalid"' <<<"$output" 2>/dev/null || echo invalid)"
    valid=false; [[ "$status" -eq 0 && "$actual" != invalid ]] && valid=true
    correct=false; [[ "$actual" == "$expected" ]] && correct=true
    jq -cn --arg model "$model" --arg id "$id" --argjson valid "$valid" --argjson correct "$correct" --argjson ms "$elapsed" \
      '{model:$model,task:$id,structured_action_valid:$valid,route_correct:$correct,complete_proposal_ms:$ms}'
  done < <(jq -r '.[] | [.id,.mode,.prompt,.expected] | @tsv' "$corpus")
done
