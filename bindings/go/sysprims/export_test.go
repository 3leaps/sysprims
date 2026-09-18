package sysprims

import "runtime"

// SetNativeCloseForTest replaces the native Close implementation. Pass nil to
// restore the real FFI close. For tests only.
func SetNativeCloseForTest(fn func(uint64) error) {
	nativeCloseFn = fn
}

// SetSnapshotHookForTest runs after receiver token loading, before native entry.
func SetSnapshotHookForTest(fn func()) { beforeSnapshotCallForTest = fn }

func ObserveFinalizerForTest(h *ContainedProcess, notify func()) {
	runtime.SetFinalizer(h, nil)
	runtime.SetFinalizer(h, func(p *ContainedProcess) { notify(); containedFinalizer(p) })
}

func RunContainedFinalizerForTest(h *ContainedProcess) { containedFinalizer(h) }
