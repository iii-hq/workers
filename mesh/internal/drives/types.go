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

package drives

import (
	"net/netip"
	"regexp"
)

type Desired struct {
	Prefix netip.Prefix `json:"prefix"`
	Serve  []Export     `json:"serve"`
	Mount  []Mount      `json:"mount"`
}
type Export struct {
	Name    string   `json:"name"`
	Clients []Client `json:"clients"`
}
type Client struct {
	Name string     `json:"name"`
	Addr netip.Addr `json:"addr"`
	SMB  *SMB       `json:"smb,omitempty"`
}
type Mount struct {
	Name   string     `json:"name"`
	Server string     `json:"server"`
	Host   string     `json:"host"`
	Addr   netip.Addr `json:"addr"`
	SMB    *SMB       `json:"smb,omitempty"`
}
type SMB struct {
	User     string `json:"user"`
	Password string `json:"password"`
}
type Result struct {
	Name  string `json:"name"`
	Path  string `json:"path,omitempty"`
	Serve bool   `json:"serve,omitempty"`
	State string `json:"state"`
	Error string `json:"error,omitempty"`
}

var driveName = regexp.MustCompile(`^[a-z0-9]+(-[a-z0-9]+)*$`)

func ValidDrive(name string) bool {
	return len(name) > 0 && len(name) <= 31 && driveName.MatchString(name)
}

var nodeName = regexp.MustCompile(`^[a-z][a-z0-9-]{0,30}$`)

func ValidNode(name string) bool { return nodeName.MatchString(name) }
