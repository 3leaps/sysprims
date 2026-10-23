# Release Signing and Verification

Official sysprims releases use GPG-signed annotated Git tags and minisign-signed
`SHA256SUMS` and `SHA512SUMS` manifests. These are separate signatures.

Establish the expected GPG primary key, the exact signing subkey, and the
minisign public-blob fingerprint through an independently trusted maintainer
channel before accepting downloaded public keys. A key distributed beside a
signature is not independent authentication.

## Public pins

| File | Contents |
| --- | --- |
| `docs/security/release-signing-keys.asc` | Public OpenPGP export: the approved primary key and the exact signing subkey, nothing else |
| `docs/security/sysprims-minisign.pub` | The minisign public key that signs the checksum manifests |
| `config/release/signing-subkey.txt` | The fingerprint of the one signing subkey tags may use |
| `config/release/tagger-identity.txt` | The exact git tagger identity |
| `keys/expected-fingerprints.txt` | Two lines: the OpenPGP primary fingerprint, then the minisign public-blob SHA-256 |
| `keys/expected-fingerprints.ndjson` | The checker's fingerprint records for the same two keys |

Release staging extracts the public keys from the verified tagged commit as
inert blobs, not from the current checkout or an external public-key locator.
The published `sysprims-minisign.pub` asset must match the committed file
byte-for-byte; fingerprint verification remains required.

## Fingerprint anchors

The text and NDJSON anchors in each tagged commit are derived by `decernor 0.1.8`
or newer. The minisign fingerprint covers the decoded public blob, not its
comment or the whole file. Verify exported public material against both anchors
with that tool, then use the approved minisign public key to verify both
manifests and check every listed digest. Require the complete expected asset
set.

Both anchors are also published with every release as assets
(`expected-fingerprints.txt`, `expected-fingerprints.ndjson`), copied from the
tagged commit's blobs and listed in both signed checksum manifests. Every
release phase requires them to equal the tagged commit's blobs byte for byte.
`make release-fetch-anchors SYSPRIMS_RELEASE_TAG=vX.Y.Z ANCHORS_DIR=<dir>`
fetches them from a published release into a new or empty directory, after
verifying the manifest signatures with the keys committed at that tag and
checking each anchor against the signed manifests and the tagged commit; it
never writes `keys/`. Those keys come from the local tag, so the local tag must
first be proven to be the signed, published one: with `SYSPRIMS_DECERNOR_BIN`
set, the command runs the remote tag verification itself; otherwise it refuses
unless `ANCHORS_TRUST_LOCAL_TAG=1` records that
`make release-verify-remote-tag` has already passed on this clone.

## Tags

A release tags one commit with each name in its tag set, at the same version:
`vX.Y.Z`, `bindings/go/sysprims/vX.Y.Z` when the Go binding is published, and
`go/recurrence/vX.Y.Z` when the recurrence Go module is present (see
[ADR-0017](../decisions/ADR-0017-recurrence-go-module-and-release-tags.md)).
Every tag must be annotated, signed by the committed signing subkey, and point
at that one commit.

Tag verification reads committed public data in an isolated keyring and requires
the exact signing subkey. The forge's own "verified" flag is an additional
check. Historical tags are not retroactively signed.

A passing tag workflow is a consistency signal; it is not release
authorization.

## Other files

- [`suppressions/`](suppressions/) records reviewed lint suppressions.
