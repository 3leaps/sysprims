// Package recurrence implements RFC 5545 recurrence rules under
// "contract: recurrence/v0": a validated rule model, canonical JSON
// (RFC 8785) and digest, canonical RFC 5545 text, and expansion into UTC
// instants.
//
// The package is pure Go (no cgo) and has no dependencies outside the
// standard library. Time zones load only from the tzdb bundle embedded in
// this module ([TZDBVersion]); the host's TZ, ZONEINFO and system zoneinfo
// are never consulted, and neither is the Go toolchain's own tzdata.
//
// A conformance claim names the pair ([CorpusDigest], [TZDBVersion]).
package recurrence

// Contract is the capability string this package implements.
const Contract = "contract: recurrence/v0"

// CorpusDigest identifies the recurrence/v0 conformance corpus this module
// is tested against (the SHA-256 of the corpus MANIFEST.json).
const CorpusDigest = "sha256:69387023c21ca147f2dd59c4e47f48447f5eb3ad77121975aab0e4fe0680b35b"
