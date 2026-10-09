# Windows release signing

EXE/DLL files are signed first by Fara Technologies LLC
(`faraCodeSigningBusiness / Faratech`), then by Mike Fara
(`fara-codesigning / MikeFara`) using append signing. Both passes use SHA256
and RFC3161 timestamps; all signatures are verified before publication.
Checksums and attestations must be generated after signing.

Authentication uses only GitHub Actions secrets: `AZURE_TENANT_ID`,
`AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`. The service principal needs the
Artifact Signing Certificate Profile Signer role on both Azure profiles.
Never commit credentials, PFX/private keys, or `.env.codesigning`.
Account/profile names and certificate subjects are public configuration.

MSIX/MSIXBundle and PowerShell scripts retain one Mike Fara signature where
already used. MSIX publisher identity is unchanged so existing installations
can update. Application executables inside the packages are dual-signed before
packaging. Microsoft Store submission identity/signing stays unchanged.

The in-app updater (`src/update_trust.rs`) only installs binaries whose primary
signature chains to CN/O "Fara Technologies LLC" through Microsoft's Trusted
Signing hierarchy ("Microsoft ID Verified Code Signing PCA 2021" and
"Microsoft Identity Verification Root Certificate Authority 2020"). Before
changing the primary signing profile or its subject, or once Microsoft moves
Trusted Signing to a new PCA or root, ship a release whose updater accepts the
new values first; otherwise installed copies refuse every later update.
