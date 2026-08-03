<!-- diataxis: reference -->

# Program contract reference

The only standalone generated-program runtime is Python 3 through contract `uhm_helper_v1`.

## Invocation

```text
python3 -I -S <trusted-launcher> <private-source-file> <one-use-contract>
```

The process runs in a private workspace with stdin closed. The trusted launcher reads and unlinks the resolved contract, resets `sys.argv`, and installs the in-memory `uhm_runtime` module before model source executes.

Generated source is standard-library only. There is no package installation, virtual environment, or dependency fetch.

## Resource modes

| Mode | Read capability | Write capability | Result behavior |
|---|---|---|---|
| `read_only` | `resource(id).read_path` | none | stdout result |
| `write_only` | none | `resource(id).write_path` | managed artifact |
| `read_write` | `resource(id).read_path` | `resource(id).write_path` | managed replacement |

Piped local input is available only when the action declares `stdin_mode=local_path`, through `uhm_runtime.stdin_path`.

The helper exposes no provider client, credential, history, shell authority, or logical writable destination.

## Preflight

Before review, the trusted AST preflight rejects:

- invalid syntax;
- `input()` or direct process-stdin access;
- undeclared resource IDs;
- literal use of declared logical paths;
- statically missing writable-resource access.

Conservative consumption findings are warnings. Preflight parses source and never executes it.

## Default limits

| Limit | Default |
|---|---:|
| Source bytes | 65,536 |
| Declared input paths | 64 |
| Declared output paths | 16 |
| Workspace bytes | 67,108,864 |
| Wall time | 10 seconds |
| CPU time | 5 seconds |
| Address space | 268,435,456 bytes |
| Open files | 64 |
| Child processes | 16 |
| Combined output | 16,777,216 bytes |
| Diagnostic tail | 1,048,576 bytes |

CPU, address-space, open-file, and child-process controls depend on host primitives. Workspace size is measured rather than enforced as a filesystem quota.

The wall deadline includes primary-process waiting, inherited stdout/stderr drainage, and bounded descendant cleanup. A descendant that keeps an inherited pipe open is classified as a timeout. Workspace measurement rejects symlinks rather than following them.

## Environment

The environment is rebuilt from locale, a minimal `PATH`, and private temporary paths. Provider keys, UHM control variables, agent sockets, and other inherited values are not intentionally passed. Arbitrary credentials require explicit `execution.deny_env` entries because their names cannot be inferred safely.

## Artifact commit

After a zero exit, every declared staged artifact must be a regular file within bounds. `uhm` checks and fsyncs each file, then renames it independently into place. A failed program commits no declared staged artifact. Multiple output renames are not one transaction, and unmanaged effects are outside the coordinator.

## Retained debugging source

Generated workspaces are removed by default. `--retain-program` keeps the private launcher and exact model source for debugging. The resolved one-use contract is unlinked before model source runs and is never retained.

## Outcomes

Program-specific failures include unavailable runtime, preflight rejection, nonzero exit, signal, timeout, spawn error, output overflow, workspace violation, and artifact-commit failure. When a child executes, its status wins under the general [behavior contract](../behavior-contract.md).
