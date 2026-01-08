# Releasing

## Release Checklist

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

| Platform | Target | Notes |
|----------|--------|-------|
| Linux | `x86_64-unknown-linux-musl` | Primary, distroless-compatible |
| Linux | `x86_64-unknown-linux-gnu` | Enterprise compatibility |
| Linux | `aarch64-unknown-linux-musl` | ARM servers |
| macOS | `x86_64-apple-darwin` | Intel Macs |
| macOS | `aarch64-apple-darwin` | Apple Silicon |
| Windows | `x86_64-pc-windows-msvc` | Windows x64 |

## Release Artifacts

```
sysprims-${VERSION}/
├── bin/
│   ├── sysprims-timeout
│   ├── sysprims-kill
│   └── sysprims-pstat
├── lib/
│   ├── libsysprims.a
│   └── libsysprims.so (Linux) / .dylib (macOS)
├── include/
│   └── sysprims.h
├── sbom-${VERSION}.spdx.json
├── THIRD_PARTY_NOTICES.md
├── LICENSE-MIT
└── LICENSE-APACHE
```

## Binding Releases

### Go

```bash
git tag bindings/go/v${VERSION}
git push origin bindings/go/v${VERSION}
# Verify: go get github.com/3leaps/sysprims/bindings/go@v${VERSION}
```

### Python

```bash
cd bindings/python
maturin build --release
twine upload dist/*
# Verify: pip install sysprims==${VERSION}
```

### TypeScript

```bash
cd bindings/typescript
npm version ${VERSION}
npm publish
# Verify: npm install @3leaps/sysprims@${VERSION}
```

## References

- [SAFETY.md](../../SAFETY.md)
- [ADR-0006: Dependency Governance](../architecture/adr/0006-dependency-governance.md)
