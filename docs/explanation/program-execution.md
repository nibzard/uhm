<!-- diataxis: explanation -->

# Why UHM generates bounded programs

Shell commands are excellent for short pipelines and existing tools. They become hard to inspect when a task needs structured parsing, statistics, several files, or careful output construction. UHM's Python route exists to keep that work readable without becoming a general coding agent.

## One program, one bounded job

The model produces one complete standard-library program with declared resources. UHM does not let it install packages, browse a repository iteratively, or continue until a broad objective is satisfied. The same global two-call ceiling and one-job lifecycle still apply.

## Capabilities instead of interpolated paths

The model declares logical resources, while trusted code resolves actual read and staging paths. This keeps path resolution and artifact publication outside model-authored source. Writable resources derive managed-artifact behavior; a model cannot obtain a writable logical destination merely by formatting a string.

## Preflight before execution

Static preflight catches contract failures such as undeclared resources, direct logical-path use, or process-stdin access before review and execution. It is intentionally conservative: clear violations are errors, while uncertain consumption findings remain warnings.

## Bounded does not mean sandboxed

Isolated Python mode, a stripped environment, private workspace, timeouts, output caps, and best-effort resource limits reduce ambient state and runaway behavior. They do not remove the operating-system authority of the current user. Generated code can still read user-readable files, access the network, and create unmanaged side effects.

This distinction is central to the review model: UHM can constrain its launcher and managed artifacts without claiming containment it does not provide.

See the [program tutorial](../tutorials/local-data.md), [program reference](../reference/program.md), and [behavior contract](../behavior-contract.md).
