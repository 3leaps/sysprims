// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build darwin

package signal

// WithForcedCheckedNumericForTest is a no-op on macOS (macOS always uses
// SignalBindingCheckedNumeric).
func WithForcedCheckedNumericForTest(fn func()) {
	fn()
}
