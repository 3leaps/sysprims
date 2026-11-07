// SPDX-License-Identifier: MIT OR Apache-2.0

package procobserve

import (
	"bytes"
	"encoding/binary"
)

// parseLinuxCmdlineArgv splits non-empty Linux /proc/<pid>/cmdline bytes per
// native-process-observation:v0 §4:
//   - If raw ends with 0x00, strip exactly one trailing 0x00 and split on 0x00.
//   - If raw does not end with 0x00 (e.g., rewritten argv stack without NUL
//     terminators), yield the single element [raw] without splitting on interior 0x00.
func parseLinuxCmdlineArgv(raw []byte) [][]byte {
	if len(raw) == 0 {
		return [][]byte{}
	}
	if raw[len(raw)-1] == 0 {
		return bytes.Split(raw[:len(raw)-1], []byte{0})
	}
	return [][]byte{raw}
}

// parseProcargs2Argv64 parses a 64-bit XNU sysctl KERN_PROCARGS2 buffer into
// argc raw argument slices.
//
// Qualified layout (macos/aarch64, macos/x86_64 with 64-bit P_LP64 target):
// XNU exec_save_path (bsd/kern/kern_exec.c) pads exec_path + '\0' to the 8-byte
// target pointer boundary (4 + alignUp8(exec_path_len + 1)) before ip_startargv.
// All bounded padding bytes must be 0x00. For argc > 0, a leading 0x00 byte at
// that boundary refuses fail-closed with "ambiguous-kern-procargs2-padding"
// rather than scanning onward across empty argv[0] into environment strings.
func parseProcargs2Argv64(buf []byte, maxBytes int) ([][]byte, string) {
	if maxBytes <= 0 || len(buf) > maxBytes {
		return nil, "arguments-exceed-cap"
	}
	if len(buf) < 4 {
		return nil, "malformed-kern-procargs2"
	}
	argcBits := binary.LittleEndian.Uint32(buf[0:4])
	if (argcBits & 0x8000_0000) != 0 {
		return nil, "negative-kern-procargs2-argc"
	}
	if uint64(argcBits) > uint64(maxBytes) {
		return nil, "arguments-exceed-cap"
	}
	argc := int(argcBits)

	nulRel := bytes.IndexByte(buf[4:], 0)
	if nulRel <= 0 {
		return nil, "malformed-kern-procargs2-execpath"
	}
	execLen := nulRel
	execEnd := 4 + execLen + 1 // one past exec_path terminating NUL

	// Checked 8-byte alignment of (execLen + 1) plus 4-byte argc prefix.
	withNul := execLen + 1
	rem := withNul % 8
	var paddedPathLen int
	if rem == 0 {
		paddedPathLen = withNul
	} else {
		paddedPathLen = withNul + (8 - rem)
	}
	alignedOff := 4 + paddedPathLen

	if argc == 0 {
		if alignedOff > len(buf) {
			return nil, "truncated-kern-procargs2-padding"
		}
		for _, b := range buf[execEnd:alignedOff] {
			if b != 0 {
				return nil, "malformed-kern-procargs2-padding"
			}
		}
		return [][]byte{}, ""
	}

	if alignedOff >= len(buf) {
		return nil, "truncated-kern-procargs2-padding"
	}
	for _, b := range buf[execEnd:alignedOff] {
		if b != 0 {
			return nil, "malformed-kern-procargs2-padding"
		}
	}
	if buf[alignedOff] == 0 {
		return nil, "ambiguous-kern-procargs2-padding"
	}

	pos := alignedOff
	argv := make([][]byte, 0, min(argc, 256))
	for i := 0; i < argc; i++ {
		if pos >= len(buf) {
			return nil, "truncated-kern-procargs2-argv"
		}
		endRel := bytes.IndexByte(buf[pos:], 0)
		if endRel < 0 {
			return nil, "unterminated-kern-procargs2-arg"
		}
		argv = append(argv, buf[pos:pos+endRel])
		pos += endRel + 1
	}
	return argv, ""
}
