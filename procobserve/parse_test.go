// SPDX-License-Identifier: MIT OR Apache-2.0

package procobserve

import (
	"bytes"
	"encoding/binary"
	"math"
	"testing"

	"github.com/3leaps/sysprims/evidence"
)

func buildProcargs2Fixture64(argc int32, execPath []byte, argv [][]byte, envTail []byte) []byte {
	var buf []byte
	var hdr [4]byte
	binary.LittleEndian.PutUint32(hdr[:], uint32(argc)) // #nosec G115 -- deliberate signed-to-wire bit pattern for negative argc tests
	buf = append(buf, hdr[:]...)
	buf = append(buf, execPath...)
	buf = append(buf, 0)
	withNul := len(execPath) + 1
	pad := (8 - (withNul % 8)) % 8
	buf = append(buf, make([]byte, pad)...)
	for _, a := range argv {
		buf = append(buf, a...)
		buf = append(buf, 0)
	}
	buf = append(buf, envTail...)
	return buf
}

func assertByteMatrixEqual(t *testing.T, got, want [][]byte) {
	t.Helper()
	if len(got) != len(want) {
		t.Fatalf("len(argv) = %d, want %d (%q vs %q)", len(got), len(want), got, want)
	}
	for i := range want {
		if !bytes.Equal(got[i], want[i]) {
			t.Fatalf("argv[%d] = %q, want %q", i, got[i], want[i])
		}
	}
}

func TestParseLinuxCmdlineArgvPreservesUnterminatedAndSplitsTerminated(t *testing.T) {
	assertByteMatrixEqual(t, parseLinuxCmdlineArgv(nil), [][]byte{})
	assertByteMatrixEqual(t, parseLinuxCmdlineArgv([]byte{}), [][]byte{})

	unterminated := []byte("alpha\x00beta")
	gotUnterm := parseLinuxCmdlineArgv(unterminated)
	assertByteMatrixEqual(t, gotUnterm, [][]byte{[]byte("alpha\x00beta")})

	terminated := []byte("alpha\x00beta\x00")
	gotTerm := parseLinuxCmdlineArgv(terminated)
	assertByteMatrixEqual(t, gotTerm, [][]byte{[]byte("alpha"), []byte("beta")})

	if evidence.DeriveArgumentsDigest(gotUnterm) == evidence.DeriveArgumentsDigest(gotTerm) {
		t.Fatal("unterminated single-element argv digest must not collide with NUL-terminated two-element argv digest")
	}

	assertByteMatrixEqual(t, parseLinuxCmdlineArgv([]byte{0}), [][]byte{{}})
	assertByteMatrixEqual(t, parseLinuxCmdlineArgv([]byte{0, 0}), [][]byte{{}, {}})
	assertByteMatrixEqual(t, parseLinuxCmdlineArgv([]byte("cmd\x00\x00")), [][]byte{[]byte("cmd"), {}})
}

func TestParseProcargs2Argv64AllEightResiduesAndEmptyArgv0Refusal(t *testing.T) {
	envTail := []byte("SECRET_ENV=NEVER_IN_ARGV\x00PATH=/usr/bin\x00\x00")

	for pathLen := 1; pathLen <= 16; pathLen++ {
		execPath := append([]byte{'/'}, bytes.Repeat([]byte{'a'}, pathLen-1)...)

		// 1. Ordinary non-empty argv[0], empty argv[1], and non-UTF-8 argv[2].
		wantArgv := [][]byte{execPath, {}, {'x', 0xff, 'y'}}
		bufOK := buildProcargs2Fixture64(3, execPath, wantArgv, envTail)
		gotArgv, reason := parseProcargs2Argv64(bufOK, MaxArgumentsRawBytes)
		if reason != "" {
			t.Fatalf("pathLen=%d unexpected refusal %q", pathLen, reason)
		}
		assertByteMatrixEqual(t, gotArgv, wantArgv)
		for _, el := range gotArgv {
			if bytes.Contains(el, []byte("SECRET_ENV")) {
				t.Fatalf("pathLen=%d leaked environment bytes into argv", pathLen)
			}
		}

		// 2. Empty argv[0] = "" must fail closed across all 8 alignment residues
		// without scanning forward into real-arg or SECRET_ENV.
		bufEmpty0 := buildProcargs2Fixture64(2, execPath, [][]byte{{}, []byte("real-arg")}, envTail)
		gotEmpty0, reasonEmpty0 := parseProcargs2Argv64(bufEmpty0, MaxArgumentsRawBytes)
		if gotEmpty0 != nil || reasonEmpty0 != "ambiguous-kern-procargs2-padding" {
			t.Fatalf("pathLen=%d empty argv[0] got (%v, %q), want (nil, ambiguous-kern-procargs2-padding)", pathLen, gotEmpty0, reasonEmpty0)
		}

		// 3. argc == 0 returns empty slice without inspecting post-padding env bytes.
		bufZero := buildProcargs2Fixture64(0, execPath, nil, envTail)
		gotZero, reasonZero := parseProcargs2Argv64(bufZero, MaxArgumentsRawBytes)
		if reasonZero != "" || len(gotZero) != 0 {
			t.Fatalf("pathLen=%d argc=0 got (%v, %q), want ([], \"\")", pathLen, gotZero, reasonZero)
		}
	}
}

func TestParseProcargs2Argv64RejectsMalformedNegativeTruncatedNonzeroPaddingAndCap(t *testing.T) {
	if _, r := parseProcargs2Argv64([]byte{0, 0, 0}, MaxArgumentsRawBytes); r != "malformed-kern-procargs2" {
		t.Fatalf("short buffer reason = %q", r)
	}

	negOne := buildProcargs2Fixture64(-1, []byte("/bin/sh"), nil, nil)
	if _, r := parseProcargs2Argv64(negOne, MaxArgumentsRawBytes); r != "negative-kern-procargs2-argc" {
		t.Fatalf("argc=-1 reason = %q", r)
	}
	negMin := buildProcargs2Fixture64(math.MinInt32, []byte("/bin/sh"), nil, nil)
	if _, r := parseProcargs2Argv64(negMin, MaxArgumentsRawBytes); r != "negative-kern-procargs2-argc" {
		t.Fatalf("argc=MinInt32 reason = %q", r)
	}

	// Empty exec_path (NUL immediately at offset 4).
	if _, r := parseProcargs2Argv64([]byte{1, 0, 0, 0, 0, 'a', 0}, MaxArgumentsRawBytes); r != "malformed-kern-procargs2-execpath" {
		t.Fatalf("empty execpath reason = %q", r)
	}
	// Unterminated exec_path.
	if _, r := parseProcargs2Argv64([]byte{1, 0, 0, 0, '/', 'b', 'i', 'n'}, MaxArgumentsRawBytes); r != "malformed-kern-procargs2-execpath" {
		t.Fatalf("unterminated execpath reason = %q", r)
	}

	// "/a\0" has withNul=3 -> requires 5 padding bytes (alignedOff = 12).
	if _, r := parseProcargs2Argv64([]byte{0, 0, 0, 0, '/', 'a', 0, 0, 0}, MaxArgumentsRawBytes); r != "truncated-kern-procargs2-padding" {
		t.Fatalf("truncated padding argc=0 reason = %q", r)
	}
	if _, r := parseProcargs2Argv64([]byte{1, 0, 0, 0, '/', 'a', 0, 0, 0, 0, 0, 0}, MaxArgumentsRawBytes); r != "truncated-kern-procargs2-padding" {
		t.Fatalf("missing argv[0] after padding reason = %q", r)
	}

	// Nonzero byte inside alignment padding.
	badPad := buildProcargs2Fixture64(1, []byte("/a"), [][]byte{[]byte("/a")}, nil)
	badPad[8] = 0x7f
	if _, r := parseProcargs2Argv64(badPad, MaxArgumentsRawBytes); r != "malformed-kern-procargs2-padding" {
		t.Fatalf("nonzero padding reason = %q", r)
	}
	badPadZero := buildProcargs2Fixture64(0, []byte("/a"), nil, nil)
	badPadZero[9] = 0x01
	if _, r := parseProcargs2Argv64(badPadZero, MaxArgumentsRawBytes); r != "malformed-kern-procargs2-padding" {
		t.Fatalf("nonzero padding argc=0 reason = %q", r)
	}

	// Truncated second argument and unterminated argument.
	truncArgv := buildProcargs2Fixture64(2, []byte("/bin/sh"), [][]byte{[]byte("/bin/sh")}, nil)
	if _, r := parseProcargs2Argv64(truncArgv, MaxArgumentsRawBytes); r != "truncated-kern-procargs2-argv" {
		t.Fatalf("truncated argv reason = %q", r)
	}
	untermArg := truncArgv[:len(truncArgv)-1]
	if _, r := parseProcargs2Argv64(untermArg, MaxArgumentsRawBytes); r != "unterminated-kern-procargs2-arg" {
		t.Fatalf("unterminated arg reason = %q", r)
	}

	// Cap checks.
	valid := buildProcargs2Fixture64(1, []byte("/bin/sh"), [][]byte{[]byte("/bin/sh")}, nil)
	if _, r := parseProcargs2Argv64(valid, 0); r != "arguments-exceed-cap" {
		t.Fatalf("maxBytes=0 reason = %q", r)
	}
	if _, r := parseProcargs2Argv64(valid, len(valid)-1); r != "arguments-exceed-cap" {
		t.Fatalf("buf > maxBytes reason = %q", r)
	}
	hugeArgc := buildProcargs2Fixture64(1024, []byte("/bin/sh"), [][]byte{[]byte("/bin/sh")}, nil)
	if _, r := parseProcargs2Argv64(hugeArgc, 64); r != "arguments-exceed-cap" {
		t.Fatalf("argc > maxBytes reason = %q", r)
	}
}
