//go:build !windows

package service

import (
	"context"
	"errors"
)

func platformPaths(p Paths) Paths                                        { return p }
func nativeInstall(string) error                                         { return errors.New("Windows service unavailable") }
func nativeUninstall() error                                             { return errors.New("Windows service unavailable") }
func RunNode(ctx context.Context, run func(context.Context) error) error { return run(ctx) }
