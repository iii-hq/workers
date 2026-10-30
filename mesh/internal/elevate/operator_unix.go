//go:build !windows

// MIT License
//
// Copyright (c) 2026 Alexander Yue
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

package elevate

import (
	"errors"
	"os"
	"os/user"
)

func Operator() (string, error) {
	current, err := user.Current()
	if err != nil {
		return "", err
	}
	return operatorName(os.Geteuid(), current.Username, os.Getenv("SUDO_USER"))
}
func operatorName(uid int, current, sudo string) (string, error) {
	name := current
	if uid == 0 {
		name = sudo
	}
	if name == "" || name == "root" {
		return "", errors.New("join requires a non-root operator; run mesh join as that user through sudo")
	}
	return name, nil
}

func Arguments(args []string) ([]string, error) { return args, nil }
