package sysprims

// SetNativeCloseForTest replaces the native Close implementation. Pass nil to
// restore the real FFI close. For tests only.
func SetNativeCloseForTest(fn func(uint64) error) {
	nativeCloseFn = fn
}
