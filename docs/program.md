# Bounded Python microprograms

Most jobs are clearest as a short shell command or pipeline, and for those `uhm` uses the shell. For structured data, statistics, or multifile logic that would become contorted in shell, it may instead generate **one standard-library Python 3 program** and run it.

This page describes how that works and where its limits end.

## When a program is chosen

The model decides between a shell command and a Python program based on the intent and context. The context it receives includes the resolved `python3` path and version, and whether `python3 -I -S` works, so it can pick a route that is actually available on your machine. `uhm doctor` reports whether the runtime is present.

A generated program is standard-library only. There is no `pip install`, no virtual environment, no network fetch of packages.

## How it runs

The program runs directly as:

```sh
python3 -I -S <private-source-file>
```

- `-I` (isolated) and `-S` (no site) reduce ambient state: no `PYTHONPATH`, no user site-packages, no `sitecustomize`.
- The environment is stripped before the child runs, so your API key and other environment values are not inherited.
- It runs in a private temporary workspace.

## Limits

The program runs under fixed limits:

| Limit | Value |
|---|---|
| Wall-clock | 10 seconds |
| CPU | 5 seconds |
| Combined output | 16 MiB |

`uhm` also applies best-effort host resource limits. These caps bound a runaway program; they are not a security boundary.

## This is not a sandbox

The program runs **with your user permissions**. A generated program can read your files, use the network, start processes, or cause unmanaged effects if its source does so. Isolated mode and resource limits reduce accidents and ambient state; they do **not** contain hostile code or protect files you can read.

Treat generated programs the way you treat any command you run: read them before you execute when it matters. `--review` shows the exact source, the manifest, detected effects, the runtime, and the limits before anything runs.

## How results get written

When a program produces files, `uhm` stages them to collision-resistant sibling paths, then commits them carefully:

1. After a zero exit, `uhm` verifies each declared artifact is a regular file.
2. It checks sizes and `fsync`s.
3. It renames each artifact into place independently.

A failed program commits **none** of its declared artifacts. But unrelated side effects the program caused — a network call, a file it wrote outside staging — cannot be rolled back. Multifile commits are not transactional.

## Keep the source for debugging

Generated workspaces are removed by default. Use `--retain-program` to keep the private temporary workspace when you want to inspect or rerun the exact source:

```sh
uhm run --retain-program tally the amount column in data.csv
```

## Next

- [Bounded recovery](recovery.md) — how to undo a program that changed a file
- [Behavior & exit codes](behavior-contract.md) — exit status for program failures
- [CLI reference](cli-reference.md) — `--retain-program` and related flags
