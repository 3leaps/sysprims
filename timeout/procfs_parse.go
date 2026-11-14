// SPDX-License-Identifier: MIT OR Apache-2.0

package timeout

import (
	"bytes"
	"strconv"
	"strings"
)

func parseLinuxStatStartTicks(stat []byte) (byte, uint64, bool) {
	endParen := bytes.LastIndexByte(stat, ')')
	if endParen < 0 || endParen+2 >= len(stat) {
		return 0, 0, false
	}
	fields := strings.Fields(string(stat[endParen+1:]))
	if len(fields) < 20 || len(fields[0]) == 0 {
		return 0, 0, false
	}
	state := fields[0][0]
	startTicks, err := strconv.ParseUint(fields[19], 10, 64)
	if err != nil {
		return 0, 0, false
	}
	return state, startTicks, true
}

func linuxProcfsMountID(fdinfo []byte) (uint64, bool) {
	for _, line := range bytes.Split(fdinfo, []byte{'\n'}) {
		if rest, ok := bytes.CutPrefix(line, []byte("mnt_id:")); ok {
			v, err := strconv.ParseUint(strings.TrimSpace(string(rest)), 10, 64)
			if err == nil {
				return v, true
			}
		}
	}
	return 0, false
}

func linuxProcfsVisibilityIsComplete(mountinfo []byte, effectiveMountID uint64) bool {
	effectiveMountIsComplete := false
	for _, line := range bytes.Split(mountinfo, []byte{'\n'}) {
		sep := bytes.Index(line, []byte(" - "))
		if sep < 0 {
			continue
		}
		mountFields := strings.Fields(string(line[:sep]))
		if len(mountFields) < 6 {
			continue
		}
		mountID, err := strconv.ParseUint(mountFields[0], 10, 64)
		if err != nil {
			continue
		}
		root := mountFields[3]
		mountPoint := mountFields[4]
		mountOptions := mountFields[5]

		fsFields := strings.Fields(string(line[sep+3:]))
		if len(fsFields) < 3 {
			continue
		}
		fsType := fsFields[0]
		superOptions := fsFields[2]

		if linuxProcfsMountMasksPID(mountPoint) {
			return false
		}
		if mountID == effectiveMountID {
			effectiveMountIsComplete = root == "/" &&
				mountPoint == "/proc" &&
				fsType == "proc" &&
				linuxProcfsOptionsAreComplete(mountOptions) &&
				linuxProcfsOptionsAreComplete(superOptions)
		}
	}
	return effectiveMountIsComplete
}

func linuxProcfsMountMasksPID(mountPoint string) bool {
	rel, ok := strings.CutPrefix(mountPoint, "/proc/")
	if !ok || rel == "" {
		return false
	}
	first := strings.SplitN(rel, "/", 2)[0]
	if first == "" {
		return false
	}
	for i := 0; i < len(first); i++ {
		if first[i] < '0' || first[i] > '9' {
			return false
		}
	}
	return true
}

func linuxProcfsOptionsAreComplete(options string) bool {
	for _, opt := range strings.Split(options, ",") {
		if strings.HasPrefix(opt, "hidepid") && opt != "hidepid=0" && opt != "hidepid=off" {
			return false
		}
	}
	return true
}

func linuxProcfsIdentityMatches(stat []byte, callerPID, callerPGID, callerSID uint32) bool {
	fields := strings.Fields(string(stat))
	if len(fields) == 0 {
		return false
	}
	procPID, err := strconv.ParseUint(fields[0], 10, 32)
	if err != nil || uint32(procPID) != callerPID {
		return false
	}
	_, pgrp, sid, ok := parseLinuxStatStateGroupSession(stat)
	return ok && pgrp == callerPGID && sid == callerSID
}

func parseLinuxStatMembership(stat []byte) (byte, uint32, bool) {
	state, pgrp, _, ok := parseLinuxStatStateGroupSession(stat)
	return state, pgrp, ok
}

func parseLinuxStatStateGroupSession(stat []byte) (byte, uint32, uint32, bool) {
	endParen := bytes.LastIndexByte(stat, ')')
	if endParen < 0 || endParen+2 >= len(stat) {
		return 0, 0, 0, false
	}
	fields := strings.Fields(string(stat[endParen+1:]))
	if len(fields) < 4 || len(fields[0]) == 0 {
		return 0, 0, 0, false
	}
	state := fields[0][0]
	pgrp, err1 := strconv.ParseUint(fields[2], 10, 32)
	sid, err2 := strconv.ParseUint(fields[3], 10, 32)
	if err1 != nil || err2 != nil {
		return 0, 0, 0, false
	}
	return state, uint32(pgrp), uint32(sid), true
}
