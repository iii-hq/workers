//go:build !windows

package meshnet

import "syscall"

const (
	errControlConnRefused = syscall.ECONNREFUSED
	errControlConnReset   = syscall.ECONNRESET
	errControlBrokenPipe  = syscall.EPIPE
)
