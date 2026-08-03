<!-- diataxis: tutorial -->

# Modify and undo a file

This tutorial walks through one managed file change and a hash-verified undo. Use a disposable file: recovery is deliberately narrow and is not a replacement for backups or version control.

## Before you start

Complete the [Quickstart](../getting-started.md). Metadata history must be enabled, which is the default.

## 1. Create a disposable file

```sh
printf 'name: Ada\nrole: engineer\n' > tutorial-profile.txt
```

## 2. Enable recovery capture

```sh
uhm recovery on
uhm recovery status
```

Read the disclosure before accepting it. Recovery snapshots duplicate eligible file contents in private local storage.

## 3. Request a managed replacement

```sh
uhm run --recoverable convert tutorial-profile.txt to compact JSON in place
```

Review the proposal if prompted. Recovery is available only when the generated program declares the destination as a managed writable resource and the original file passes eligibility checks.

## 4. Inspect the evidence

```sh
uhm history show last
uhm recovery status last
```

The recovery status should describe an available snapshot and the managed output state.

## 5. Undo the change

```sh
uhm undo last
cat tutorial-profile.txt
```

`undo` rechecks the current postimage and retained preimage before restoring. If you edit the file after step 3, it reports a conflict instead of overwriting your newer work.

## 6. Turn capture off

```sh
uhm recovery off
uhm recovery prune --dry-run
```

Use `uhm recovery off --prune` only when you also want to remove eligible retained snapshots.

## What you learned

Verified undo depends on captured evidence and current hashes, not an inferred inverse command. See [Recover prior work](../how-to/recover-work.md) for operational variants and the [recovery model](../explanation/recovery-model.md) for the design rationale.
