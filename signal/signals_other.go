// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build !darwin

package signal

const (
	// SIGUSR1 is User-defined signal 1 on Linux/Windows/POSIX catalog default (10).
	SIGUSR1 = 10
	// SIGUSR2 is User-defined signal 2 on Linux/Windows/POSIX catalog default (12).
	SIGUSR2 = 12
)
