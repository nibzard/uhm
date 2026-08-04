#!/usr/bin/env python3
"""Build a deterministic fixture corpus for the uhm evaluation.

Emits expected.json with exact counts/sums/names so the battery runner can make
machine-checkable assertions instead of hardcoding magic numbers.
"""
import os, json, csv, subprocess, collections, textwrap

ROOT = os.environ.get("UHM_EVAL_ROOT", "/dev/shm/uhm-eval")
WORK = f"{ROOT}/work"
os.makedirs(f"{WORK}/docs", exist_ok=True)
os.makedirs(f"{WORK}/src", exist_ok=True)
os.makedirs(f"{WORK}/blobs", exist_ok=True)
# isolated runtime dirs
for d in ("home", "config", "data", "cache", "out"):
    os.makedirs(f"{ROOT}/{d}", exist_ok=True)

E = {}

# --- NOTES.md : exactly 5 blank-line-separated paragraphs ---
paragraphs = [
    "Uhm is a natural-language layer over terminal tools. You describe a small job in words and it picks one bounded action.",
    "Each job runs a single shell command or one generated Python program. The result is real output, not a chat reply.",
    "The default provider is OpenAI. The Cerebras adapter is available as an explicit alternative for low latency.",
    "Privacy is scoped by default: standard context sends OS, shell, tool booleans, and bounded directory names, never file contents.",
    "History is private metadata by default. Telemetry is content-free and opt-out is always honored by the runtime.",
]
notes = "\n\n".join(paragraphs) + "\n"
open(f"{WORK}/NOTES.md", "w").write(notes)
E["notes_paragraphs"] = len(paragraphs)
E["notes_words"] = len(notes.split())

# --- README.md ---
readme = textwrap.dedent("""\
    # work

    Fixture corpus for the uhm evaluation harness.

    It holds markdown, a csv table, a json object, a yaml config, a log file, and
    a small source tree. Use it as the working directory for natural-language jobs.
""")
open(f"{WORK}/README.md", "w").write(readme)
E["readme_words"] = len(readme.split())

# --- docs/*.md (for concatenation task) ---
open(f"{WORK}/docs/design.md", "w").write("# design\n\nOne intent in, one bounded job out.\n")
open(f"{WORK}/docs/limits.md", "w").write("# limits\n\nNo open-ended chat loop. The tool exits after one job.\n")

# --- events.log : 60 lines, known status distribution ---
codes = [200, 200, 200, 404, 500, 404, 200, 403]
lines, sc = [], collections.Counter()
for i in range(60):
    code = codes[i % len(codes)]
    sc[str(code)] += 1
    lines.append(f"2026-08-0{1+i%3}T10:{i%60:02d}:00Z req={i:03d} status={code} path=/item/{i} ms={20+i}")
open(f"{WORK}/events.log", "w").write("\n".join(lines) + "\n")
E["log_lines"] = len(lines)
E["log_status_counts"] = dict(sorted(sc.items()))
E["log_error_lines"] = sc["404"] + sc["500"] + sc["403"]

# --- sales.csv : region,item,amount — known sum/mean and per-region sums ---
rows, amounts, by_region = [], [], collections.defaultdict(int)
regions = ["north", "south", "east", "west"]
for i in range(40):
    region = regions[i % 4]
    amount = (i + 1) * 100 + (i % 3) * 5  # deterministic, no floats
    rows.append([region, f"item-{i:02d}", amount])
    amounts.append(amount)
    by_region[region] += amount
with open(f"{WORK}/sales.csv", "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["region", "item", "amount"])
    w.writerows(rows)
E["sales_rows"] = len(rows)
E["sales_amount_sum"] = sum(amounts)
E["sales_amount_mean"] = sum(amounts) / len(amounts)
E["by_region"] = dict(sorted(by_region.items()))

# --- data.json : known top-level keys ---
data = {"alpha": 1, "beta": "two", "gamma": {"x": 1, "y": 2}, "delta": [1, 2, 3]}
open(f"{WORK}/data.json", "w").write(json.dumps(data, indent=2) + "\n")
E["data_json_keys"] = sorted(data.keys())

# --- config.yaml ---
open(f"{WORK}/config.yaml", "w").write("region: north\nretries: 3\nendpoint: https://example.local\n")

# --- source tree with TODO markers ---
open(f"{WORK}/src/parser.rs", "w").write(textwrap.dedent("""\
    // entry point for the line parser
    pub fn parse(input: &str) -> usize {
        // TODO: handle empty input safely
        input.lines().count()
    }
"""))
open(f"{WORK}/src/loader.py", "w").write(textwrap.dedent("""\
    def load(path):
        # TODO: add a read cache
        with open(path) as fh:
            return fh.read()
"""))
E["todo_in_rust"] = 1  # parser.rs has one TODO

# --- nested dirs + sized files so "largest" has a known winner ---
open(f"{WORK}/big.bin", "wb").write(b"x" * (500 * 1024))            # 500 KiB — guaranteed largest
open(f"{WORK}/b.txt", "w").write("b" * (8 * 1024))                  # 8 KiB
open(f"{WORK}/a.txt", "w").write("a" * (4 * 1024))                  # 4 KiB
open(f"{WORK}/blobs/small.txt", "w").write("s" * 100)

# --- git repo with 2 commits (for git-state tasks) ---
subprocess.run(["git", "init", "-q"], cwd=WORK, check=True)
subprocess.run(["git", "config", "user.email", "eval@uhm.local"], cwd=WORK, check=True)
subprocess.run(["git", "config", "user.name", "uhm-eval"], cwd=WORK, check=True)
open(f"{WORK}/.gitignore", "w").write("*.tmp\n*.done\ncombined.md\nnow.txt\ntotal.txt\nkeys.txt\nstatus_counts.json\nby_region.csv\nmarker.*\nunder_review.*\n")
subprocess.run(["git", "add", "NOTES.md", "README.md", ".gitignore"], cwd=WORK, check=True)
subprocess.run(["git", "commit", "-q", "-m", "seed docs"], cwd=WORK, check=True)
subprocess.run(["git", "add", "events.log"], cwd=WORK, check=True)
subprocess.run(["git", "commit", "-q", "-m", "add events log"], cwd=WORK, check=True)
E["git_commits"] = 2
E["git_branch"] = subprocess.run(["git", "rev-parse", "--abbrev-ref", "HEAD"],
                                 cwd=WORK, capture_output=True, text=True).stdout.strip()

# --- expected "largest"/"top3" computed over non-.git files ---
sizes = []
for dp, dns, fns in os.walk(WORK):
    if ".git" in dp.split(os.sep):
        continue
    for fn in fns:
        p = os.path.join(dp, fn)
        sizes.append((os.path.relpath(p, WORK), os.path.getsize(p)))
sizes.sort(key=lambda t: (-t[1], t[0]))
E["largest_file"] = sizes[0][0]
E["top3_files"] = [name for name, _ in sizes[:3]]
E["all_files"] = [name for name, _ in sizes]

with open(f"{ROOT}/expected.json", "w") as f:
    json.dump(E, f, indent=2)

print("corpus built at", WORK)
print("files:", len(E["all_files"]))
print("expected.json written:", sorted(E.keys()))
print("  notes_paragraphs =", E["notes_paragraphs"], "| notes_words =", E["notes_words"])
print("  log_lines =", E["log_lines"], "| log_status_counts =", E["log_status_counts"])
print("  sales_rows =", E["sales_rows"], "| amount_sum =", E["sales_amount_sum"], "| mean =", round(E["sales_amount_mean"], 2))
print("  data_json_keys =", E["data_json_keys"])
print("  largest_file =", E["largest_file"], "| top3 =", E["top3_files"])
