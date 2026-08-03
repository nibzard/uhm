#!/usr/bin/env python3
"""Generate the checked-in 120-task Plan 9 corpus deterministically."""

from __future__ import annotations

import csv
import hashlib
import io
import json
from pathlib import Path
import re
import shlex

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "tests/fixtures/provider-execution-benchmark-v2.json"
REFERENCE_OUT = ROOT / "tests/fixtures/provider-execution-reference-actions-v4.json"


def re_escape(value): return re.escape(value)
def sh_quote(value): return shlex.quote(value)


def shell(command, summary, effects, requirements, stdin_mode="none"):
    return {"tool":"run_shell","arguments":{"command":command,"summary":summary,"assumptions":[],"effects":effects,"requirements":requirements,"stdin_mode":stdin_mode}}


def program(source, summary, inputs, outputs=(), result="stdout"):
    effects = ["read_local"] + (["write_local"] if outputs else [])
    files=[]; by_path={}; input_rows=[]; output_rows=[]; stdin_mode="none"
    for index,item in enumerate(inputs):
        if item["path"]=="stdin":
            stdin_mode="local_path"; input_rows.append("{'path':str(_stdin_path),'access':'read_only'}"); continue
        ident=f"input_{index+1}"; access="read_write" if item["access"]=="replace" else "read_only"
        files.append({"id":ident,"path":item["path"],"access":access}); by_path[item["path"]]=ident
        input_rows.append(f"{{'path':str(_resource({ident!r}).read_path),'access':{item['access']!r}}}")
    for index,path in enumerate(outputs):
        ident=by_path.get(path)
        if ident is None:
            ident=f"output_{index+1}"; files.append({"id":ident,"path":path,"access":"write_only"})
        output_rows.append(f"{{'path':str(_resource({ident!r}).write_path)}}")
    bridge=("import json as _json, os as _os\nfrom uhm_runtime import stdin_path as _stdin_path, resource as _resource\n"
            f"_os.environ['UHM_PROGRAM_INPUTS']=_json.dumps([{','.join(input_rows)}],separators=(',',':'))\n"
            f"_os.environ['UHM_PROGRAM_OUTPUTS']=_json.dumps([{','.join(output_rows)}],separators=(',',':'))\n"
            "if _stdin_path is not None: _os.environ['UHM_PROGRAM_LOCAL_INPUT']=str(_stdin_path)\n")
    return {"tool":"run_program","arguments":{"runtime":"python3","contract":"uhm_helper_v1","source":bridge+source,"summary":summary,"assumptions":[],"stdin_mode":stdin_mode,"files":files,"effects":effects}}


def parent(kind, path=None, name=None, value=None):
    return {"tool":"require_parent_shell","arguments":{"kind":kind,"path":path,"name":name,"value":value,"summary":"Apply the requested current-shell change","assumptions":[],"effects":["shell_state"]}}


def answer(text): return {"tool":"return_answer","arguments":{"text":text}}
def clarify(question): return {"tool":"request_clarification","arguments":{"question":question}}


def negative(reference):
    tool = reference["tool"]
    if tool == "run_shell":
        return shell("true", "Intentionally incorrect no-op", [], ["true"])
    if tool == "run_program":
        result=json.loads(json.dumps(reference)); args=result["arguments"]
        writable=[item["id"] for item in args["files"] if item["access"] in {"write_only","read_write"}]
        if writable:
            args["source"] += "\nfrom uhm_runtime import resource as _negative_resource\n" + "\n".join(f"_negative_resource({ident!r}).write_path.write_text('wrong')" for ident in writable) + "\n"
        else:
            args["source"] += "\nprint('wrong')\n"
        args["summary"]="Intentionally incorrect result"
        return result
    if tool == "require_parent_shell":
        return parent("change_directory", path=".")
    if tool == "return_answer":
        return answer("This is unrelated.")
    return clarify("Could you clarify?")


def task(task_id, tool, prompt, category, fixture, expected, reference, rubric, difficulty="medium"):
    return {
        "id":task_id,"mode":"ask" if tool in {"return_answer", "request_clarification"} else "run","prompt":prompt,
        "expected_tools":[tool],"rubric":rubric,"judge_rubric":rubric,
        "tags":{"expected_tool":tool,"category":category,"difficulty":difficulty,"effects":reference["arguments"].get("effects",[])},
        "fixture":fixture,"limits":{"wall_ms":10000,"stdout_bytes":1048576,"stderr_bytes":262144,"workspace_bytes":67108864},
        "expected":expected,"reference_action":reference,
        "negative_action":negative(reference)
    }


def fixture(files=None, stdin=None, directories=None, environment=None, git=None):
    return {"cwd":"/work","stdin":stdin,"directories":directories or [],"files":files or [],"symlinks":[],"environment":environment or {},"git":git}


def text_file(path, text, mode="0644"): return {"path":path,"text":text,"mode":mode}
def expected(stdout=None, filesystem=None, exit_codes=(0,), forbid=True, parent_state=None):
    return {"exit_codes":list(exit_codes),"stdout":stdout or {"matcher":"empty"},"stderr":{"matcher":"empty"},"filesystem":filesystem or [],"forbid_undeclared_changes":forbid,"parent_state":parent_state}


def stdin(text, fmt="text/plain"): return {"encoding":"utf-8","declared_format":fmt,"text":text}


tasks=[]

# 28 shell read/search/inspect tasks: seven families, four fixtures each.
for i in range(4):
    files=[text_file(f"data/f{i}-{j}.txt","x"*(j*7+i+1)) for j in range(1,8)]
    sizes=sorted([(len(f["text"].encode()),f["path"]) for f in files],reverse=True)[:5]
    pattern="^"+"".join(f"{size}[ \\t]+\\./{re_escape(path)}\\n" for size,path in sizes)+"$"
    tasks.append(task(f"shell-largest-{i+1:02}","run_shell","list the five largest regular files recursively, largest first","shell-read",fixture(files),expected({"matcher":"regex","pattern":pattern}),shell("find . -type f -printf '%s %p\\n' | sort -nr -k1,1 | head -n 5","List five largest files",["read_local"],["find","sort","head"]),"Recurse over regular files, sort by byte size, and return exactly five entries."))
for i in range(4):
    files=[text_file(f"src/mod{i}.rs",f"fn main() {{}}\n// TODO: item {i}\n"),text_file(f"src/clean{i}.rs","fn clean() {}\n"),text_file("notes.txt","TODO not Rust\n")]
    out=f"./src/mod{i}.rs:2:// TODO: item {i}\n"
    tasks.append(task(f"shell-rust-todo-{i+1:02}","run_shell","find TODO comments in Rust files recursively with filenames and line numbers","shell-read",fixture(files),expected({"matcher":"exact_text","value":out}),shell("grep -RIn --include='*.rs' 'TODO' .","Find Rust TODO comments",["read_local"],["grep"]),"Restrict to Rust files, recurse, and include filename and line number."))
extensions=["json","csv","log","md"]
for i,ext in enumerate(extensions):
    files=[text_file(f"a.{ext}","a"),text_file(f"nested/b.{ext}","b"),text_file("nested/no.txt","n")]
    tasks.append(task(f"shell-count-{ext}","run_shell",f"count regular .{ext} files recursively","shell-read",fixture(files),expected({"matcher":"integer_equals","value":2}),shell(f"find . -type f -name '*.{ext}' | wc -l",f"Count .{ext} files",["read_local"],["find","wc"]),"Count only regular files with the requested extension recursively."))
for i,limit in enumerate([10,20,30,40]):
    files=[text_file(f"files/f{j}.bin","z"*(j*10+1)) for j in range(1,6)]
    names=[f"./files/f{j}.bin" for j in range(1,6) if j*10+1>limit]
    tasks.append(task(f"shell-threshold-{limit}","run_shell",f"print regular files larger than {limit} bytes, sorted by path","shell-read",fixture(files),expected({"matcher":"exact_text","value":"".join(x+"\n" for x in names)}),shell(f"find . -type f -size +{limit}c -print | sort",f"Find files larger than {limit} bytes",["read_local"],["find","sort"]),"Use the exact byte threshold, regular files, recursion, and stable path order."))
for i in range(4):
    name=f"archive-{i}.tar.gz"; content=f"archive payload {i}\n"
    digest=hashlib.sha256(content.encode()).hexdigest()
    files=[text_file(name,content),text_file("ignore.zip","x")]
    tasks.append(task(f"shell-sha256-{i+1:02}","run_shell","print SHA-256 checksums for every .tar.gz file in this directory","shell-read",fixture(files),expected({"matcher":"regex","pattern":f"^{digest}  (?:\\./)?{re_escape(name)}\\n$"}),shell("find . -maxdepth 1 -type f -name '*.tar.gz' -exec sha256sum -- {} +","Hash tar.gz files",["read_local"],["find","sha256sum"]),"Hash exactly the matching regular files and handle operands safely."))
git_states=[
    ({},"## main\n"),
    ({"untracked":{"new.txt":"new\n"}},"## main\n?? new.txt\n"),
    ({"unstaged":{"tracked.txt":"changed\n"}},"## main\n M tracked.txt\n"),
    ({"staged":{"added.txt":"added\n"}},"## main\nA  added.txt\n"),
]
for i,(state,out) in enumerate(git_states):
    git={"branch":"main","commits":[{"message":"initial","files":{"tracked.txt":"base\n"}}],**state}
    status_lines=out.splitlines(); branch=status_lines[0].removeprefix("## ")
    tasks.append(task(f"shell-git-status-{i+1:02}","run_shell","show the current git branch and concise working tree status","shell-read",fixture(git=git),expected({"matcher":"git_status","value":{"branch":branch,"lines":status_lines[1:]}}),shell("git status --short --branch","Show branch and status",["read_local"],["git"]),"Report the branch and staged, unstaged, or untracked state accurately."))
for i in range(4):
    lines=["pear","apple","pear","banana","apple","pear"][:3+i]
    counts={x:lines.count(x) for x in set(lines)}
    out="".join(f"{counts[x]} {x}\n" for x in sorted(counts))
    command="sort | uniq -c | awk '{print $1 \" \" $2}'"
    tasks.append(task(f"shell-frequency-{i+1:02}","run_shell","count repeated input lines and print count plus line alphabetically","shell-read",fixture(stdin=stdin("\n".join(lines)+"\n")),expected({"matcher":"count_map","value":counts}),shell(command,"Count repeated lines",["read_local"],["sort","uniq","awk"],"original"),"Consume original stdin and produce correct sorted counts."))

# 20 bounded shell mutations.
names=["Quarterly Reports","--literal","café-data","nested/output"]
for i,name in enumerate(names):
    tasks.append(task(f"shell-mkdir-{i+1:02}","run_shell",f"create a directory named {name}","shell-write",fixture(),expected(filesystem=[{"path":name,"state":"directory"}]),shell(f"mkdir -p -- {sh_quote(name)}",f"Create {name}",["write_local"],["mkdir"]),"Create exactly the named directory with safe operand handling."))
for i,name in enumerate(["empty.txt","--draft","résumé.txt","space name.txt"]):
    tasks.append(task(f"shell-touch-{i+1:02}","run_shell",f"create an empty file named {name}","shell-write",fixture(),expected(filesystem=[{"path":name,"state":"file","content":""}]),shell(f"touch -- {sh_quote(name)}",f"Create {name}",["write_local"],["touch"]),"Create exactly one empty file and preserve the literal name."))
for i in range(3):
    src=f"source {i}.txt"; dst=f"copy {i}.txt"; content=f"payload {i}\n"
    tasks.append(task(f"shell-copy-{i+1:02}","run_shell",f"copy '{src}' to '{dst}'","shell-write",fixture([text_file(src,content)]),expected(filesystem=[{"path":dst,"state":"file","content":content}]),shell(f"cp -- {sh_quote(src)} {sh_quote(dst)}",f"Copy {src}",["read_local","write_local"],["cp"]),"Copy the exact source to the exact destination without changing the source."))
for i in range(3):
    src=f"old-{i}.txt"; dst=f"new-{i}.txt"; content=f"move {i}\n"
    tasks.append(task(f"shell-move-{i+1:02}","run_shell",f"rename {src} to {dst}","shell-write",fixture([text_file(src,content)]),expected(filesystem=[{"path":src,"state":"absent"},{"path":dst,"state":"file","content":content}]),shell(f"mv -- {sh_quote(src)} {sh_quote(dst)}",f"Rename {src}",["write_local","delete_local"],["mv"]),"Move exactly the named file and preserve its content."))
for i in range(3):
    name=f"delete-{i}.tmp"
    tasks.append(task(f"shell-delete-file-{i+1:02}","run_shell",f"delete {name}","shell-write",fixture([text_file(name,"temporary")]),expected(filesystem=[{"path":name,"state":"absent"}]),shell(f"rm -- {sh_quote(name)}",f"Delete {name}",["delete_local"],["rm"]),"Delete exactly the named file without a glob or broader path."))
for i in range(3):
    name=f"build/cache-{i}"; files=[text_file(f"{name}/a.txt","a"),text_file(f"{name}/nested/b.txt","b"),text_file("keep.txt","keep")]
    tasks.append(task(f"shell-delete-tree-{i+1:02}","run_shell",f"delete the directory {name} and everything inside it","shell-write",fixture(files),expected(filesystem=[{"path":name,"state":"absent"},{"path":"keep.txt","state":"file","content":"keep"}]),shell(f"rm -rf -- {sh_quote(name)}",f"Delete {name}",["delete_local"],["rm"]),"Recursively delete only the exact requested subtree."))

# 28 Python stdout tasks (seven families x four fixtures).
SRC_JSON_TOTAL="""import json, os\np=json.loads(open(json.loads(os.environ['UHM_PROGRAM_INPUTS'])[0]['path'],encoding='utf-8').read())\nr={}\nfor x in p:r[x['category']]=r.get(x['category'],0)+x['amount']\nprint(json.dumps(r,sort_keys=True))\n"""
SRC_CSV_COUNT="""import csv, json, os\np=json.loads(os.environ['UHM_PROGRAM_INPUTS'])[0]['path']\nwith open(p,newline='',encoding='utf-8') as f: print(sum(1 for _ in csv.DictReader(f)))\n"""
SRC_PRETTY="""import json, os\np=json.loads(os.environ['UHM_PROGRAM_INPUTS'])[0]['path']\nwith open(p,encoding='utf-8') as f: obj=json.load(f)\nprint(json.dumps(obj,sort_keys=True,indent=2))\n"""
SRC_WORDS="""import collections, json, os, re\np=json.loads(os.environ['UHM_PROGRAM_INPUTS'])[0]['path']\ntext=open(p,encoding='utf-8').read().lower()\nfor word,count in sorted(collections.Counter(re.findall(r'[a-z]+',text)).items()): print(f'{word} {count}')\n"""
SRC_FILTER="""import json, os\np=json.loads(os.environ['UHM_PROGRAM_INPUTS'])[0]['path']\nobj=json.load(open(p,encoding='utf-8'))\nprint(json.dumps([x for x in obj if x.get('active')],sort_keys=True))\n"""
SRC_AVG="""import csv, json, os\np=json.loads(os.environ['UHM_PROGRAM_INPUTS'])[0]['path']\nwith open(p,newline='',encoding='utf-8') as f: vals=[float(x['score']) for x in csv.DictReader(f)]\nprint(f'{sum(vals)/len(vals):.2f}')\n"""
SRC_STATS="""import json, os\np=json.loads(os.environ['UHM_PROGRAM_INPUTS'])[0]['path']\nlines=open(p,encoding='utf-8').read().splitlines()\nprint(json.dumps({'lines':len(lines),'nonempty':sum(bool(x.strip()) for x in lines),'characters':sum(len(x) for x in lines)},sort_keys=True))\n"""
for i in range(4):
    data=[{"category":"b","amount":i+2},{"category":"a","amount":3},{"category":"b","amount":5}]
    value={"a":3,"b":i+7}
    tasks.append(task(f"program-json-total-{i+1:02}","run_program","sum amount by category from piped JSON and print a sorted JSON object","program-stdout",fixture(stdin=stdin(json.dumps(data),"application/json")),expected({"matcher":"json_equals","value":value}),program(SRC_JSON_TOTAL,"Sum category amounts",[{"path":"stdin","access":"read_only"}]),"Parse structured JSON, aggregate correctly, and write valid sorted JSON to stdout."))
for i in range(4):
    rows=[("name","score")]+[(f"user{j}",str(j)) for j in range(i+1, i+4)]
    text="\n".join(",".join(r) for r in rows)+"\n"
    tasks.append(task(f"program-csv-count-{i+1:02}","run_program","count data rows in the piped CSV, excluding the header","program-stdout",fixture(stdin=stdin(text,"text/csv")),expected({"matcher":"integer_equals","value":3}),program(SRC_CSV_COUNT,"Count CSV rows",[{"path":"stdin","access":"read_only"}]),"Use the CSV parser and exclude exactly one header row."))
for i in range(4):
    obj={"z":i,"a":{"d":4,"c":3}}; out=json.dumps(obj,sort_keys=True,indent=2)+"\n"
    tasks.append(task(f"program-json-pretty-{i+1:02}","run_program","pretty-print piped JSON with sorted keys and two-space indentation","program-stdout",fixture(stdin=stdin(json.dumps(obj),"application/json")),expected({"matcher":"exact_text","value":out}),program(SRC_PRETTY,"Pretty-print JSON",[{"path":"stdin","access":"read_only"}]),"Parse and serialize JSON with exact indentation and key ordering."))
for i in range(4):
    words=("Apple banana apple " + "pear "*i).strip()+"\n"; counts={"apple":2,"banana":1};
    if i: counts["pear"]=i
    out="".join(f"{k} {v}\n" for k,v in sorted(counts.items()))
    tasks.append(task(f"program-word-count-{i+1:02}","run_program","print case-insensitive word counts alphabetically from piped text","program-stdout",fixture(stdin=stdin(words)),expected({"matcher":"count_map","value":counts}),program(SRC_WORDS,"Count words",[{"path":"stdin","access":"read_only"}]),"Tokenize words case-insensitively and produce correct stable counts."))
for i in range(4):
    obj=[{"id":j,"active":j%2==i%2} for j in range(1,5)]; selected=[x for x in obj if x["active"]]
    tasks.append(task(f"program-json-filter-{i+1:02}","run_program","print a JSON array containing only objects whose active field is true","program-stdout",fixture(stdin=stdin(json.dumps(obj),"application/json")),expected({"matcher":"json_equals","value":selected}),program(SRC_FILTER,"Filter active objects",[{"path":"stdin","access":"read_only"}]),"Filter by the boolean field and emit valid JSON."))
for i in range(4):
    vals=[i+1,i+2,i+6]; text="name,score\n"+"\n".join(f"u{j},{v}" for j,v in enumerate(vals))+"\n"; out=f"{sum(vals)/3:.2f}\n"
    tasks.append(task(f"program-csv-average-{i+1:02}","run_program","compute the average score from piped CSV and print two decimal places","program-stdout",fixture(stdin=stdin(text,"text/csv")),expected({"matcher":"exact_text","value":out}),program(SRC_AVG,"Average CSV scores",[{"path":"stdin","access":"read_only"}]),"Parse CSV numerically and format the correct average."))
for i in range(4):
    lines=["alpha","",f"item{i}"]; text="\n".join(lines)+"\n"; value={"lines":3,"nonempty":2,"characters":len("alpha")+len(f"item{i}")}
    tasks.append(task(f"program-line-stats-{i+1:02}","run_program","print JSON with line count, nonempty line count, and characters excluding newlines","program-stdout",fixture(stdin=stdin(text)),expected({"matcher":"json_equals","value":value}),program(SRC_STATS,"Compute line statistics",[{"path":"stdin","access":"read_only"}]),"Compute all requested statistics with the declared newline convention."))

# 20 Python artifact tasks (five families x four fixtures).
SRC_SORT_FILE="""import json, os\ni=json.loads(os.environ['UHM_PROGRAM_INPUTS'])[0]['path']; o=json.loads(os.environ['UHM_PROGRAM_OUTPUTS'])[0]['path']\nobj=json.load(open(i,encoding='utf-8'))\nopen(o,'w',encoding='utf-8').write(json.dumps(obj,sort_keys=True,indent=2)+'\\n')\n"""
SRC_CSV_JSON="""import csv,json,os\ni=json.loads(os.environ['UHM_PROGRAM_INPUTS'])[0]['path']; o=json.loads(os.environ['UHM_PROGRAM_OUTPUTS'])[0]['path']\nwith open(i,newline='',encoding='utf-8') as f: rows=list(csv.DictReader(f))\nopen(o,'w',encoding='utf-8').write(json.dumps(rows,sort_keys=True,indent=2)+'\\n')\n"""
SRC_UPPER="""import json,os\ni=json.loads(os.environ['UHM_PROGRAM_INPUTS'])[0]['path']; o=json.loads(os.environ['UHM_PROGRAM_OUTPUTS'])[0]['path']\nopen(o,'w',encoding='utf-8').write(open(i,encoding='utf-8').read().upper())\n"""
SRC_MERGE="""import json,os\nins=json.loads(os.environ['UHM_PROGRAM_INPUTS']); o=json.loads(os.environ['UHM_PROGRAM_OUTPUTS'])[0]['path']; result={}\nfor item in ins: result.update(json.load(open(item['path'],encoding='utf-8')))\nopen(o,'w',encoding='utf-8').write(json.dumps(result,sort_keys=True)+'\\n')\n"""
SRC_JSONL="""import json,os\ni=json.loads(os.environ['UHM_PROGRAM_INPUTS'])[0]['path']; o=json.loads(os.environ['UHM_PROGRAM_OUTPUTS'])[0]['path']\nwith open(i,encoding='utf-8') as src, open(o,'w',encoding='utf-8') as dst:\n for line in src:\n  obj=json.loads(line)\n  if obj.get('keep'): dst.write(json.dumps(obj,sort_keys=True)+'\\n')\n"""
for i in range(4):
    obj={"z":i,"a":{"y":2,"b":1}}; content=json.dumps(obj); wanted=json.dumps(obj,sort_keys=True,indent=2)+"\n"
    ref=program(SRC_SORT_FILE,"Sort JSON keys",[{"path":"settings.json","access":"replace"}],["settings.json"],"artifacts")
    tasks.append(task(f"program-sort-file-{i+1:02}","run_program","rewrite settings.json in place with recursively sorted keys and two-space indentation","program-artifact",fixture([text_file("settings.json",content)]),expected(filesystem=[{"path":"settings.json","state":"file","content":wanted}]),ref,"Use a managed replacement and produce the exact valid JSON artifact."))
for i in range(4):
    text=f"name,score\nAda,{i+1}\nGrace,{i+2}\n"; rows=[{"name":"Ada","score":str(i+1)},{"name":"Grace","score":str(i+2)}]; wanted=json.dumps(rows,sort_keys=True,indent=2)+"\n"
    ref=program(SRC_CSV_JSON,"Convert CSV to JSON",[{"path":"input.csv","access":"read_only"}],["output.json"],"artifacts")
    tasks.append(task(f"program-csv-json-{i+1:02}","run_program","convert input.csv to a pretty JSON array in output.json","program-artifact",fixture([text_file("input.csv",text)]),expected(filesystem=[{"path":"output.json","state":"file","content":wanted}]),ref,"Parse CSV and create the requested managed JSON artifact."))
for i in range(4):
    text=f"Hello café {i}\n"; ref=program(SRC_UPPER,"Uppercase text",[{"path":"input.txt","access":"read_only"}],["upper.txt"],"artifacts")
    tasks.append(task(f"program-uppercase-{i+1:02}","run_program","write an uppercase copy of input.txt to upper.txt","program-artifact",fixture([text_file("input.txt",text)]),expected(filesystem=[{"path":"upper.txt","state":"file","content":text.upper()}]),ref,"Preserve Unicode and create only the requested output."))
for i in range(4):
    a={"a":i,"shared":"first"}; b={"b":i+1,"shared":"second"}; wanted=json.dumps({**a,**b},sort_keys=True)+"\n"
    ref=program(SRC_MERGE,"Merge JSON objects",[{"path":"a.json","access":"read_only"},{"path":"b.json","access":"read_only"}],["merged.json"],"artifacts")
    tasks.append(task(f"program-merge-json-{i+1:02}","run_program","merge a.json then b.json into merged.json, with later keys winning","program-artifact",fixture([text_file("a.json",json.dumps(a)),text_file("b.json",json.dumps(b))]),expected(filesystem=[{"path":"merged.json","state":"file","content":wanted}]),ref,"Merge in the requested order and create valid sorted JSON."))
for i in range(4):
    objs=[{"id":j,"keep":j%2==i%2} for j in range(4)]; text="".join(json.dumps(x)+"\n" for x in objs); wanted="".join(json.dumps(x,sort_keys=True)+"\n" for x in objs if x["keep"])
    ref=program(SRC_JSONL,"Filter JSONL",[{"path":"events.jsonl","access":"read_only"}],["kept.jsonl"],"artifacts")
    tasks.append(task(f"program-filter-jsonl-{i+1:02}","run_program","write records with keep=true from events.jsonl to kept.jsonl","program-artifact",fixture([text_file("events.jsonl",text)]),expected(filesystem=[{"path":"kept.jsonl","state":"file","content":wanted}]),ref,"Parse each JSONL record and produce only matching records."))

# Eight typed parent-shell actions.
parent_cases=[
 ("cd-tmp","change_directory","change my current shell directory to /tmp",{"path":"/tmp"},parent("change_directory",path="/tmp"),{"cwd":"/tmp"}),
 ("cd-work","change_directory","change my current shell directory to /work/subdir",{"directories":["subdir"]},parent("change_directory",path="/work/subdir"),{"cwd":"/work/subdir"}),
 ("set-rust-log","set_environment","set RUST_LOG to debug in my current shell",{},parent("set_environment",name="RUST_LOG",value="debug"),{"environment":{"RUST_LOG":"debug"}}),
 ("set-space-value","set_environment","set REPORT_TITLE to Quarterly Reports in my current shell",{},parent("set_environment",name="REPORT_TITLE",value="Quarterly Reports"),{"environment":{"REPORT_TITLE":"Quarterly Reports"}}),
 ("unset-http-proxy","unset_environment","unset HTTP_PROXY in my current shell",{"environment":{"HTTP_PROXY":"http://proxy"}},parent("unset_environment",name="HTTP_PROXY"),{"environment_absent":["HTTP_PROXY"]}),
 ("unset-debug","unset_environment","unset DEBUG_MODE in my current shell",{"environment":{"DEBUG_MODE":"1"}},parent("unset_environment",name="DEBUG_MODE"),{"environment_absent":["DEBUG_MODE"]}),
 ("source-env","source_file","source env.sh in my current shell",{"files":[text_file("env.sh","export FROM_ENV=loaded\n")]},parent("source_file",path="/work/env.sh"),{"environment":{"FROM_ENV":"loaded"}}),
 ("source-space","source_file","source scripts/my env.sh in my current shell",{"files":[text_file("scripts/my env.sh","export SPACE_ENV=yes\n")]},parent("source_file",path="/work/scripts/my env.sh"),{"environment":{"SPACE_ENV":"yes"}}),
]
for ident,kind,prompt,fx,ref,state in parent_cases:
    tasks.append(task(f"parent-{ident}","require_parent_shell",prompt,"parent-shell",fixture(files=fx.get("files"),directories=fx.get("directories"),environment=fx.get("environment")),expected(parent_state=state),ref,"Return the exact typed parent-shell action without shell source."))

# Eight clarifications and eight prose answers.
clarifications=[
 ("missing-input","convert the report to CSV","Which report should I convert?",r"(?is).*(report|file).*"),
 ("missing-output","save the results","Where should I save the results?",r"(?is).*(where|path|destination|location|filename).*"),
 ("missing-delimiter","split this data into columns","What delimiter separates the columns?",r"(?is).*(delimiter|separator).*"),
 ("missing-scope","delete the old files","Which files or directory define the old-file scope?",r"(?is).*(which|scope|directory).*"),
 ("missing-overwrite","write the cleaned data back","Which input file should be replaced?",r"(?is).*(which|file|replace|overwrite).*"),
 ("missing-encoding","decode this file","What file and encoding should I use?",r"(?is).*(encoding|charset).*"),
 ("missing-format","export the data","What output format should I use?",r"(?is).*(format|type).*"),
 ("missing-destination","copy the backup","Where should I copy the backup?",r"(?is).*(where|destination|path).*"),
]
for ident,prompt,question,pattern in clarifications:
    tasks.append(task(f"clarify-{ident}","request_clarification",prompt,"clarification",fixture(),expected({"matcher":"regex","pattern":pattern}),clarify(question),"Ask one concise question for the smallest essential missing fact."))
answers=[
 ("git-log-p","what does git log -p do","It shows commit history together with the patch introduced by each commit.",r"(?is).*(patch|diff).*commit.*"),
 ("pipefail","what does set -o pipefail change","It makes a pipeline fail when an earlier command fails instead of usually reporting only the final command's status.",r"(?is).*pipeline.*fail.*"),
 ("git-status-short","what does git status --short show","It shows a compact two-column summary of staged and working-tree changes.",r"(?is).*(compact|short).*changes.*"),
 ("find-print0","why use find -print0 with xargs -0","They use NUL delimiters so filenames containing whitespace or newlines are passed safely.",r"(?is).*(NUL|null).*(filename|whitespace|newline).*"),
 ("chmod-600","what does chmod 600 secrets do","It gives the owner read and write permission while removing permissions for group and others.",r"(?is).*owner.*read.*write.*(group|others).*"),
 ("python-isolated","what does python3 -I do","It runs Python in isolated mode, ignoring user site packages and Python-related environment variables.",r"(?is).*isolated.*(user|environment).*"),
 ("git-detached","what is a detached HEAD in git","HEAD points directly to a commit rather than to a branch name.",r"(?is).*HEAD.*commit.*(branch|reference).*"),
 ("stderr","why do commands write diagnostics to stderr","It keeps diagnostics separate from normal stdout so output can be piped or captured independently.",r"(?is).*diagnostic.*(stdout|output).*"),
]
for ident,prompt,text,pattern in answers:
    tasks.append(task(f"answer-{ident}","return_answer",prompt,"answer",fixture(),expected({"matcher":"regex","pattern":pattern}),answer(text),"Give a concise and factually correct explanation without proposing local execution."))

assert len(tasks)==120, len(tasks)
counts={}
for index,t in enumerate(tasks):
    counts[t["tags"]["category"]]=counts.get(t["tags"]["category"],0)+1
    match=re.match(r"^(.*)-(\d+)$",t["id"])
    t["family_id"]=match.group(1) if match else t["id"]
    t["variant_id"]=match.group(2) if match else "base"
    t["split"]="development"
    preferred=t.pop("expected_tools")[0]
    executable=preferred in {"run_shell","run_program"}
    t["route_oracle"]={
        "allowed":["run_shell","run_program"] if executable else [preferred],
        "preferred":preferred,
        "rationale":"Both bounded executable routes may satisfy this outcome; the preferred route is simpler for this task." if executable else "This task requires the typed semantic route.",
    }
    reference=t.pop("reference_action")
    alternate=json.loads(json.dumps(reference))
    if preferred=="run_shell":
        alternate["arguments"]["command"]="set -e; "+alternate["arguments"]["command"]
    elif preferred=="run_program":
        alternate["arguments"]["source"]="# equivalent reference implementation\n"+alternate["arguments"]["source"]
    t["reference_actions"]=[reference] if not executable else [reference,alternate]
    t["negative_actions"]=[t.pop("negative_action")]
    t["oracle_disposition"]="Generated reference and targeted negative were verified offline; judge disagreements require a separate recorded audit disposition."
document={"version":2,"prompt_version":9,"action_schema_version":4,"worker_contract_version":2,"reference_bundle":"provider-execution-reference-actions-v4.json","task_count":120,"family_count":len({t['family_id'] for t in tasks}),"route_counts":counts,"tasks":tasks}
OUT.parent.mkdir(parents=True,exist_ok=True)
OUT.write_text(json.dumps(document,indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
REFERENCE_OUT.write_text(json.dumps({"version":4,"action_schema_version":4,"program_contract":"uhm_helper_v1","tasks":[{"id":t["id"],"reference_actions":t["reference_actions"],"negative_actions":t["negative_actions"]} for t in tasks]},indent=2,ensure_ascii=False)+"\n",encoding="utf-8")
print(OUT)
print(REFERENCE_OUT)
