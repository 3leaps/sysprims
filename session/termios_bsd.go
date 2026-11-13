// SPDX-License-Identifier: MIT OR Apache-2.0

//go:build darwin || freebsd || netbsd || openbsd

package session

import "golang.org/x/sys/unix"

const ioctlGetTermiosReq = unix.TIOCGETA
