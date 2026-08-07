# Release signing (`uhm update` authenticity)

## Why

A `uhm` release publishes a `SHA256SUMS` manifest alongside the platform
archives, and the installer checks every downloaded archive against it. On its
own that manifest is **self-attested**: whoever can place a file on the release
(lost GitHub credentials, a compromised maintainer token, or a TLS-stripping
man-in-the-middle against a manual `curl install.sh | sh`) can ship a `SHA256SUMS`
that matches a tampered binary and the check still passes.

Release signing closes that gap. The updater carries a pinned public key and
verifies a detached **minisign (Ed25519)** signature over `SHA256SUMS` before it
trusts the manifest. A forged or stripped signature cannot substitute a matching
checksum for a malicious binary.

## What the updater does

`uhm update` (in `src/update.rs`) implements this policy:

1. If the compiled-in key is still the placeholder, verification is **not
   configured**. The update proceeds over TLS with the checksum check, and the
   updater prints a warning naming this document. This keeps the default build
   working until a key is in place.
2. Once a real key is compiled in, verification is **fail-closed**. The updater
   fetches `SHA256SUMS` and `SHA256SUMS.minisig` through uhm's own rustls agent,
   verifies the signature under the pinned key, and stages the *authenticated*
   bytes for the installer. If the signature is missing, oversized, malformed,
   or fails to verify, the update is refused.
3. The installer (`docs/install.sh`) still downloads the archive over TLS and
   checks it against the authenticated manifest, so transport and authenticity
   are both enforced.

The first time you install uhm (`curl ... | sh`) is a trust bootstrap: there is
no binary yet to carry a pinned key, so it is checksum-plus-TLS only. Every
`uhm update` after that is signature-checked once a key is configured.

## Enabling signed releases (maintainer, one-time)

1. **Generate a keypair.** Use the `-W` flag so the secret key is stored
   unencrypted and CI can sign without a password prompt (minisign has no
   non-interactive password input; without `-W` it always prompts interactively,
   which would fail in a TTY-less runner):

   ```sh
   minisign -G -W -p uhm-release.pub -s uhm-release.key
   ```

   The unencrypted key is protected by GitHub's encrypted secret store and lives
   on the runner only transiently during the release job (step 3 deletes it).

2. **Publish the public key into the binary.** `uhm-release.pub` has a comment
   line and a base64 line; the base64 line is the key:

   ```sh
   tail -n 1 uhm-release.pub
   ```

   Paste that single line as the value of `RELEASE_PUBLIC_KEY` in
   `src/update.rs` (replacing `RELEASE_PUBLIC_KEY_PLACEHOLDER`). Commit it.

3. **Store the secret key as a repository secret.** GitHub secrets hold text, so
   base64-encode the binary key file first:

   ```sh
   base64 -w0 uhm-release.key   # GNU coreutils; on macOS: base64 < uhm-release.key | tr -d '\n'
   ```

   Copy the output into a new repository secret named
   `UHM_RELEASE_SIGNING_KEY`.

4. **Back the secret key up offline.** The GitHub secret cannot be read back
   after it is set. If you lose `uhm-release.key` you cannot sign further
   releases with the same key and must rotate (below).

5. **Tag the next release.** The release workflow installs `minisign` and signs
   `SHA256SUMS` into `SHA256SUMS.minisig` whenever `UHM_RELEASE_SIGNING_KEY` is
   present. The release now ships nine assets (eight plus the signature).

You can confirm a published signature locally:

```sh
minisign -V -m SHA256SUMS -x SHA256SUMS.minisig -P "$(tail -n 1 uhm-release.pub)"
```

## Verifying authenticity from the compiled key

Anyone can extract the pinned public key from the installed binary's source
history and verify a release signature against it with `minisign -V ... -P <key>`
or `minisign-verify`. The key is public; only `uhm-release.key` is secret.

## Rotating the key

A pinned key cannot be replaced silently: older binaries carry the old key and
will reject signatures under a new one. The rotation therefore needs one
transition release that old binaries still accept while it installs the new key.

The release workflow signs `SHA256SUMS` with a single secret
(`UHM_RELEASE_SIGNING_KEY`) and cross-checks that signature against the single
key compiled into `src/update.rs`. A true rotation — a transition release signed
with the **old** key while the binary carries the **new** key — does not fall out
of that single-secret, single-key pipeline automatically and needs a one-time
accommodation for that release. For example, keep `UHM_RELEASE_SIGNING_KEY` on
the old secret for the transition release so it signs with the old key, and
verify the cross-check against both the old and new keys rather than the new one
alone (or produce the old-key signature out of band and upload it after the
workflow, since `gh release upload --clobber` would otherwise overwrite it).

Finalize the exact mechanism when you first activate signing. It is not exercised
while the placeholder key is compiled in, and it only matters once a real key
exists and a rotation is actually needed. After the transition release, set
`RELEASE_PUBLIC_KEY` to the new public key and `UHM_RELEASE_SIGNING_KEY` to the
base64 of the new secret; every release after that is signed with the new key,
and old binaries that accepted the transition release now pin it.

If the old key is lost or compromised and no transition release was prepared,
users must reinstall from a trusted channel to pick up the new key — there is no
in-band recovery for a pinned-key failure. This is the standard trade-off of
pinned-key signing.

## Manual `curl install.sh | sh` installs

The standalone installer cannot carry a compiled-in key, so it does not verify
the signature on its own (it cannot do Ed25519 in POSIX `sh`). It still enforces
TLS plus the SHA-256 check. To get signature verification, install once and then
run `uhm update`, which performs the signature check described above.
