# Releasing

## Release Checklist

- [ ] **Run preflight**: `make release-preflight` (REQUIRED before tagging)
- [ ] All CI green on Linux/macOS/Windows
- [ ] Changelog updated
- [ ] Schema versions updated if needed (meta-validated)
- [ ] Third-party notices generated (`cargo about generate`)
- [ ] SBOM generated (`cargo sbom`)
- [ ] Artifacts built for all target platforms
- [ ] Checksums published

## Versioning

- Crates and published packages use SemVer
- Workspace version in root `Cargo.toml` is SSOT
- If CalVer tags are used, they supplement (not replace) SemVer

## Build Artifacts

| Platform | Target                       | Notes                          |
| -------- | ---------------------------- | ------------------------------ |
| Linux    | `x86_64-unknown-linux-musl`  | Primary, distroless-compatible |
| Linux    | `x86_64-unknown-linux-gnu`   | Enterprise compatibility       |
| Linux    | `aarch64-unknown-linux-musl` | ARM servers                    |
| macOS    | `x86_64-apple-darwin`        | Intel Macs                     |
| macOS    | `aarch64-apple-darwin`       | Apple Silicon                  |
| Windows  | `x86_64-pc-windows-msvc`     | Windows x64                    |

## Release Artifacts

GitHub release assets are flat and covered by the signed checksum manifests
(see [Release Asset Policy](../standards/release-asset-policy.md)):

- CLI archives (`sysprims-${VERSION}-<platform>.tar.gz`, `.zip` on Windows)
- SBOM (`sysprims-${VERSION}.cdx.json`)
- Python `cp310-abi3` wheels and sdist
- npm tarballs (`3leaps-sysprims*-${VERSION}.tgz`)
- `LICENSE-MIT`, `LICENSE-APACHE`

No native libraries or C headers are shipped; the C-ABI and the cgo Go
bindings module were retired in v0.5.0.

## Package Releases

The authoritative procedure is [`RELEASE_CHECKLIST.md`](../../RELEASE_CHECKLIST.md).

### Go

The root module (`github.com/3leaps/sysprims`) resolves from the signed
`v${VERSION}` tag, and `go/recurrence` from the signed
`go/recurrence/v${VERSION}` tag on the same commit:

```bash
# Verify:
go list -m github.com/3leaps/sysprims@v${VERSION}
go list -m github.com/3leaps/sysprims/go/recurrence@v${VERSION}
```

### Python and TypeScript

`sysprims` (PyPI) and `@3leaps/sysprims` (npm) are published with Trusted
Publishing (OIDC) from the signed GitHub Release assets, without rebuilding:

```bash
# Verify:
pip install sysprims==${VERSION}
npm install @3leaps/sysprims@${VERSION}
```

## References

- [SAFETY.md](../../SAFETY.md)
- [ADR-0006: Dependency Governance](../architecture/adr/0006-dependency-governance.md)
