// SPDX-License-Identifier: MIT OR Apache-2.0

package timeout

import "runtime"

// SetNativeCloseForTest overrides the low-level close hook in tests.
func SetNativeCloseForTest(fn func(uint64) error) {
	if fn == nil {
		nativeCloseFn.Store((func(uint64) error)(nil))
		return
	}
	nativeCloseFn.Store(fn)
}

// SetSnapshotHookForTest installs a pre-snapshot callback in tests.
func SetSnapshotHookForTest(fn func()) {
	if fn == nil {
		beforeSnapshotCallForTest.Store((func())(nil))
		return
	}
	beforeSnapshotCallForTest.Store(fn)
}

// RunContainedFinalizerForTest invokes the finalizer directly in tests.
func RunContainedFinalizerForTest(handle *ContainedProcess) {
	containedFinalizer(handle)
}

// ObserveFinalizerForTest chains fn after the finalizer on handle in tests.
func ObserveFinalizerForTest(handle *ContainedProcess, fn func()) {
	runtime.SetFinalizer(handle, nil)
	runtime.SetFinalizer(handle, func(target *ContainedProcess) {
		containedFinalizer(target)
		if fn != nil {
			fn()
		}
	})
}
