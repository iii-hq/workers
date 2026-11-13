package meshnet

import "golang.org/x/sys/windows"

// Windows' POSIX-named errno constants are invented values, not Winsock errors.
const (
	errControlConnRefused = windows.WSAECONNREFUSED
	errControlConnReset   = windows.WSAECONNRESET
	errControlBrokenPipe  = windows.ERROR_BROKEN_PIPE
)
