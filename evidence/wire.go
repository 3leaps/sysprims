// SPDX-License-Identifier: MIT OR Apache-2.0

package evidence

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"io"

	"github.com/3leaps/canonprims/canon"
)

const (
	// MaxReportBytes is the maximum received report or evidence payload size (256 KiB).
	MaxReportBytes = 262_144
	// MaxJSONDepth is the maximum JSON container nesting depth (outermost container == 1).
	MaxJSONDepth = 16
	// MaxCanonicalBytes is the maximum RFC 8785 canonical JSON output size (256 KiB).
	MaxCanonicalBytes = 262_144
)

// AdmittedWire holds the RFC 8785 canonical bytes, sha256:<hex> digest, and
// parsed JSON value of an admitted wire document.
type AdmittedWire struct {
	CanonicalBytes []byte
	Digest         string
	Value          any
}

// WireRefusal reports a normative report-admission:v0 or canonprims refusal code.
type WireRefusal struct {
	Code string
}

// Error implements error.
func (w *WireRefusal) Error() string {
	return w.Code
}

func wireLimits() canon.Limits {
	l := canon.DefaultLimits()
	l.MaxBytes = MaxReportBytes
	l.MaxDepth = MaxJSONDepth
	return l
}

func sha256Prefixed(b []byte) string {
	sum := sha256.Sum256(b)
	return "sha256:" + hex.EncodeToString(sum[:])
}

// AdmitReportBytes enforces report-admission:v0 over raw UTF-8 JSON bytes:
//  1. Raw byte cap (> 262,144 -> "report-too-large") prior to parsing;
//  2. Strict UTF-8, RFC 7493 I-JSON, duplicate-member, binary64 integer,
//     finite-number, and depth <= 16 checks via canon.Canonicalize(canon.JCS);
//  3. Canonical output cap (> 262,144 -> "canonical-report-too-large");
//  4. sha256:<lowercase hex> over the RFC 8785 canonical bytes.
func AdmitReportBytes(raw []byte) (AdmittedWire, error) {
	if len(raw) > MaxReportBytes {
		return AdmittedWire{}, &WireRefusal{Code: "report-too-large"}
	}
	canonicalBytes, err := canon.Canonicalize(canon.JCS, raw, wireLimits())
	if err != nil {
		var ref canon.Refusal
		if errors.As(err, &ref) {
			if ref.Code == canon.TooLarge {
				return AdmittedWire{}, &WireRefusal{Code: "report-too-large"}
			}
			return AdmittedWire{}, &WireRefusal{Code: string(ref.Code)}
		}
		return AdmittedWire{}, &WireRefusal{Code: "invalid_json"}
	}
	if len(canonicalBytes) > MaxCanonicalBytes {
		return AdmittedWire{}, &WireRefusal{Code: "canonical-report-too-large"}
	}
	var val any
	if err := json.Unmarshal(canonicalBytes, &val); err != nil {
		return AdmittedWire{}, &WireRefusal{Code: "invalid_json"}
	}
	return AdmittedWire{
		CanonicalBytes: canonicalBytes,
		Digest:         sha256Prefixed(canonicalBytes),
		Value:          val,
	}, nil
}

// AdmitReportReader enforces report-admission:v0 over an io.Reader with bounded
// reading (io.LimitReader(r, MaxReportBytes+1)).
func AdmitReportReader(r io.Reader) (AdmittedWire, error) {
	buf, err := io.ReadAll(io.LimitReader(r, MaxReportBytes+1))
	if err != nil {
		return AdmittedWire{}, &WireRefusal{Code: "io_error"}
	}
	if len(buf) > MaxReportBytes {
		return AdmittedWire{}, &WireRefusal{Code: "report-too-large"}
	}
	return AdmitReportBytes(buf)
}

// CanonicalSHA256Value computes sha256:<lowercase hex> over the RFC 8785
// canonical JSON form of a validated JSON value.
func CanonicalSHA256Value(v any) (string, error) {
	raw, err := json.Marshal(v)
	if err != nil {
		return "", &WireRefusal{Code: "invalid_json"}
	}
	dig, err := canon.Digest(canon.JCS, raw, canon.DefaultLimits(), "sha256")
	if err != nil {
		var ref canon.Refusal
		if errors.As(err, &ref) {
			return "", &WireRefusal{Code: string(ref.Code)}
		}
		return "", &WireRefusal{Code: "invalid_json"}
	}
	return dig, nil
}
