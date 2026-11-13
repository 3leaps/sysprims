// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build !darwin && !linux && !windows

package signal

import (
	"runtime"

	"github.com/3leaps/sysprims/syserror"
)

// WithForcedCheckedNumericForTest is a no-op on non-Linux platforms.
func WithForcedCheckedNumericForTest(fn func()) {
	fn()
}

type platformProcessHandle struct{}

func openPlatformHandle(_ uint32, _ string) (string, SignalBinding, *platformProcessHandle, error) {
	return "", "", nil, syserror.NotSupported("open_process_handle", runtime.GOOS)
}

func (*platformProcessHandle) closeHandle() {}

func (*platformProcessHandle) checkExited(_ uint32, _ string) (bool, error) {
	return false, syserror.NotSupported("open_process_handle", runtime.GOOS)
}

func (*platformProcessHandle) sendSignal(_ uint32, _ string, _ int) (SignalBinding, error) {
	return "", syserror.NotSupported("open_process_handle", runtime.GOOS)
}
