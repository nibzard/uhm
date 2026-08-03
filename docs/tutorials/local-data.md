<!-- diataxis: tutorial -->

# Process local data without sending its contents

In this tutorial you will total a CSV column while keeping the CSV body on your machine. The selected provider receives the task, the input size and encoding status, and the format label—but not the rows.

## Before you start

Complete the [Quickstart](../getting-started.md) first. You need a configured provider, a working `python3 -I -S`, and a terminal.

## 1. Create a small input file

```sh
printf 'item,amount\nalpha,12\nbeta,30\n' > tutorial-data.csv
```

## 2. Inspect the outbound shape

```sh
uhm context show minimal
```

This prints the context structure without making a provider request. The actual local-input request will add only input presence, byte count, UTF-8 status, and the format label.

## 3. Run the local-input job

```sh
cat tutorial-data.csv | uhm --local-input --input-format text/csv \
  total the amount column
```

`--local-input` requires piped input. If the provider returns the bounded Python route, the private program reads the spool through `uhm_runtime.stdin_path`. A shell route cannot receive the local-only body and is rejected by the contract.

## 4. Confirm the result

The result should report `42`. The CSV remains an ordinary local file, and the private spool is removed after the job.

## 5. Compare with ordinary piped input

Without `--local-input`, explicitly piped UTF-8 bytes are part of the provider request:

```sh
cat tutorial-data.csv | uhm ask summarize these rows
```

Use that form only when sending the body is intentional.

## What you learned

You used metadata to let the provider choose a compatible local program while keeping the actual data on-device. For exact limits and failure behavior, see the [program reference](../reference/program.md) and [privacy contract](../privacy.md).
