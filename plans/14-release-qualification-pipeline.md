# Plan 14 — Make model qualification release-ready

Status: offline pipeline implemented; paid qualification blocked until an independently authored holdout is reviewed and sealed.

## Goal

Turn Plan 13's evidence policy into a fail-closed release process. Development measurements must never qualify automatic routing, and runtime evidence must be reproducible from one sealed holdout, completed blinded audit, and deterministic manifest.

## Delivered foundation

- A distinct holdout commitment binds the private corpus, reference bundle, frozen policy, reviewer, and seal time before candidate results exist.
- `--profile qualification` requires first-shot mode, two candidates, three trials, an all-holdout corpus, exact source fingerprints, and no task filtering.
- Qualification computes every frozen point, Wilson, equal-family bootstrap, per-stratum, paired non-inferiority, calibration, identity, audit, and scope gate.
- A strict 20-item blinded audit pauses finalization and rejects material or critical adjudications.
- Manifest generation revalidates the finalized artifact and current runtime compatibility inputs, then asks the production runtime to validate the generated manifest.
- Evidence selection is unavailable unless exactly one compatible selected entry exists for a pre-call request class.
- The development corpus and current unavailable commitment cannot produce production qualification.

## External release gate

1. Have an independent author prepare the private all-holdout corpus and schema-v4 reference bundle.
2. Have a second person review independence, references, negative examples, oracles, and at least 60 targeted scope families.
3. Seal and commit the commitment before any candidate call.
4. Run the paid holdout once, complete the generated 20-item blinded audit, and resume without changing inputs.
5. Generate and independently review the runtime manifest, check it in, and rerun every offline release gate.

The exact commands and custody rules are in [`docs/qualification.md`](../docs/qualification.md). No agent working from the public development corpus may synthesize, relabel, or inspect the private holdout on behalf of the independent author.

## Completion criteria

- Offline implementation is complete when Rust tests and strict Clippy, Python unit/containment tests, Node tests, package construction, and qualification fail-closed checks pass.
- Automatic evidence routing remains deliberately unavailable until the external release gate completes.
- A paid run is not a prerequisite for merging the fail-closed pipeline; it is a prerequisite for checking in any qualified runtime entry.
