// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build darwin || linux

package procobserve

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"

	"github.com/3leaps/sysprims/evidence"
	"golang.org/x/sys/unix"
)

func isLowercaseUUID(s string) bool {
	if len(s) != 36 {
		return false
	}
	for i := 0; i < 36; i++ {
		if i == 8 || i == 13 || i == 18 || i == 23 {
			if s[i] != '-' {
				return false
			}
		} else if !isLowerHex(s[i]) {
			return false
		}
	}
	return true
}

func factObserved(val string) map[string]any {
	return map[string]any{
		"status": "observed",
		"value":  val,
	}
}

func factNonObserved(status, reason string) map[string]any {
	return map[string]any{
		"status": status,
		"reason": reason,
	}
}

func startIdentityObserved(profileID, val string) map[string]any {
	return map[string]any{
		"status": "observed",
		"profile": map[string]any{
			"id":       profileID,
			"revision": evidence.NativeProcessObservationReadmeRevision,
		},
		"value": val,
	}
}

func startIdentityNonObserved(profileID, status, reason string) map[string]any {
	return map[string]any{
		"status": status,
		"profile": map[string]any{
			"id":       profileID,
			"revision": evidence.NativeProcessObservationReadmeRevision,
		},
		"reason": reason,
	}
}

func principalObserved(derivationID, namespace, value string) map[string]any {
	return map[string]any{
		"status": "observed",
		"derivation": map[string]any{
			"id":       derivationID,
			"revision": evidence.NativeProcessObservationReadmeRevision,
		},
		"principal": map[string]any{
			"namespace": namespace,
			"value":     value,
		},
	}
}

func digestObserved(derivationID, digest string) map[string]any {
	return map[string]any{
		"status": "observed",
		"derivation": map[string]any{
			"id":       derivationID,
			"revision": evidence.NativeProcessObservationReadmeRevision,
		},
		"digest": digest,
	}
}

func errnoToDigestFact(err error, fallbackReason string) map[string]any {
	if errors.Is(err, unix.EPERM) || errors.Is(err, unix.EACCES) {
		return factNonObserved("inaccessible", "permission-denied")
	}
	if errors.Is(err, unix.ENOENT) || errors.Is(err, unix.ESRCH) {
		return factNonObserved("not-present", "backing-entry-absent")
	}
	return factNonObserved("error", fallbackReason)
}

func streamExecutableDigestFromFD(fd int) map[string]any {
	var st unix.Stat_t
	if err := unix.Fstat(fd, &st); err != nil {
		return errnoToDigestFact(err, "executable-fstat-failed")
	}
	if (st.Mode & unix.S_IFMT) != unix.S_IFREG {
		return factNonObserved("error", "executable-not-regular-file")
	}
	if st.Size < 0 {
		return factNonObserved("error", "executable-negative-size")
	}
	if st.Size > MaxExecutableFileBytes {
		return factNonObserved("error", "executable-exceeds-size-cap")
	}

	h := sha256.New()
	_, _ = h.Write([]byte("3leaps.native-process-observation.executable-image.v0\x00"))

	buf := make([]byte, 65_536)
	var total int64
	for {
		n, err := unix.Read(fd, buf)
		if n > 0 {
			total += int64(n)
			if total > MaxExecutableFileBytes {
				return factNonObserved("error", "executable-exceeds-size-cap")
			}
			_, _ = h.Write(buf[:n])
		}
		if err != nil {
			if errors.Is(err, unix.EINTR) {
				continue
			}
			return errnoToDigestFact(err, "executable-read-failed")
		}
		if n == 0 {
			break
		}
	}

	digest := "sha256:" + hex.EncodeToString(h.Sum(nil))
	return digestObserved(ExecDigestDerivationID, digest)
}
