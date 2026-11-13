// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build linux || android

package session

import "golang.org/x/sys/unix"

const ioctlGetTermiosReq = unix.TCGETS
