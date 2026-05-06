# Release signing

Inferra release artifacts can be signed in CI when the right credentials and OIDC permissions are available.

## Container images (cosign, keyless)

The `Release` workflow signs the pushed multi-arch image digest with [cosign](https://docs.sigstore.dev/cosign/overview/) in keyless mode (`cosign sign --yes`) using the GitHub Actions OIDC token. The image reference uses `ghcr.io` plus the repository slug lowercased for GHCR naming rules. Requirements:

- `permissions: id-token: write` and `contents: read` (or `write` if pushing packages) on the workflow job.
- No local key file: cosign exchanges the OIDC token for a short-lived signing certificate.

If OIDC is unavailable in a fork, the sign step fails; remove or gate that step for fork builds.

## Windows executable (signtool, Authenticode)

When a code-signing certificate is installed in the Windows runner certificate store, set `WINDOWS_CERT_THUMBPRINT` to the SHA-1 thumbprint of the signing cert. The release job runs:

`signtool sign /sha1 %WINDOWS_CERT_THUMBPRINT% /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 dist/inferra.exe`

When the secret is absent, the job skips signing and uploads the unsigned `inferra.exe` artifact. For production releases, sign on a trusted machine or use a managed signing service.

## SBOM

Python packages are summarized with `cyclonedx-bom` (`cyclonedx-py environment`) and uploaded as `sbom-cyclonedx` artifact JSON.
