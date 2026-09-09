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
