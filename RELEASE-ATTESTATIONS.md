# Existing release verification attestations

The September 9, 2026 Windows release replacements were re-signed locally without
rebuilding. Their original build attestations do not cover the replacement hashes.
The manually dispatched `attest-existing-release.yml` workflow verifies their
SHA256 hashes against the reviewed, committed manifest and verifies Authenticode
signatures with `signtool verify /pa /all /v`. It then issues a custom attestation
for those exact bytes using GitHub Actions OIDC; no Azure signing secrets are used.

EXE/DLL signatures, including archive members, must be Fara Technologies LLC
followed by Mike Fara. MSIX and MSIX bundle containers retain Mike Fara as publisher.
Checksum manifest subjects are hash-checked only. Archives are inspected without
executing application code. A changed release hash fails verification and requires
a separately reviewed manifest update; this workflow never modifies release assets.

Verify a downloaded asset (replace FILE with its filename):

```sh
gh attestation verify FILE --repo faratech/htop-win --predicate-type https://faratech.dev/attestations/windows-release-verification/v1 --signer-workflow faratech/htop-win/.github/workflows/attest-existing-release.yml
```

This checks the attestation signature, subject digest, predicate type and issuing
workflow. Use `--format json` to inspect the verified predicate and confirm its
releaseTag and verification results. The custom URI identifies schema version 1:
repository, releaseTag, verifiedAt, workflowCommit, runUrl, assets (name/sha256),
signatureChecks (path/check/publishers), and limitations.

This is evidence of hash and signature verification, **not build provenance**:
it does not prove compilation or source-to-binary correspondence. Default
`gh attestation verify` expects SLSA build provenance, so use the explicit
predicate type above. Future normal build workflows retain their build attestations.
