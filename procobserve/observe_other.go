// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build !darwin && !linux

package procobserve

import (
	"runtime"
	"time"
)

type platformHandle struct{}

func (*platformHandle) closeHandle() error {
	return nil
}

func (*platformHandle) waitExit(_ time.Duration) (bool, error) {
	return false, unqualifiedPlatformRefusal()
}

func unqualifiedPlatformRefusal() *CollectorRefusal {
	if runtime.GOOS == "windows" {
		return newRefusal("unsupported",
			"windows-collector-not-qualified",
			"Windows start-identity profile (windows-filetime:v0) is reserved and not yet qualified",
		)
	}
	return newRefusal("unsupported",
		"unsupported-platform",
		"No native-process-observation:v0 collector is qualified for this target OS",
	)
}

func observeProcessPlatform(_ uint32, _ map[string]any, _ ObservationOptions) (map[string]any, error) {
	return nil, unqualifiedPlatformRefusal()
}

func openContinuityHandlePlatform(_ uint32, _ map[string]any, _ ObservationOptions) (map[string]any, *platformHandle, error) {
	return nil, nil, unqualifiedPlatformRefusal()
}
