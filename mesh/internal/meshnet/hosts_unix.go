//go:build !windows

package meshnet

func platformHostsPath() string { return "/etc/hosts" }
