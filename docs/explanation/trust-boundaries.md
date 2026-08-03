<!-- diataxis: explanation -->

# Trust boundaries

UHM separates model suggestions, trusted validation, local execution, retained evidence, and aggregate telemetry. Understanding those boundaries explains why some features are deliberately strict or inconvenient.

## Provider output is untrusted

Each provider adapter parses its own wire format, but no adapter can authorize work directly. One canonical local decoder enforces the closed action set, semantic bounds, route constraints, and runtime preflight. Provider-specific schema adaptation cannot weaken local acceptance.

## Execution uses your authority

Commands and generated programs run as your user. Warnings, review, isolated Python mode, resource limits, and staging reduce mistakes; they are not a security sandbox. The trusted boundary is the UHM control path, not every effect produced by child code.

## Writable artifacts are mediated

Generated programs receive private staging paths rather than logical writable destinations. UHM validates and commits declared artifacts after successful execution. Unmanaged effects remain possible and are not rollback-safe.

## Local records have separate purposes

History is an inspectable local decision journal. Recovery snapshots are separately consented file evidence. The cache stores validated proposals. Telemetry receives only a fixed content-free projection. These stores are not interchangeable, and richer local retention never silently broadens telemetry.

## Outbound authorization is disclosed

The first-use notice names the selected fixed provider endpoint set and telemetry boundary before outbound work. Changing a cross-provider alternate changes that authorized set and requires disclosure again. Provider-side retention remains governed by each provider's terms.

## Qualification is a release boundary

Development measurements can improve the system but cannot authorize automatic selection. Evidence mode accepts only reviewed, fingerprint-bound qualification artifacts and otherwise returns unavailable.

For normative details, see the [privacy contract](../privacy.md), [behavior contract](../behavior-contract.md), and [provider reference](../reference/providers.md).
