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
	"errors"
	"regexp"
	"strings"
)

var netUseLine = regexp.MustCompile(`(?m)(?:^|[ \t])([A-Za-z]:)[ \t]+(\\\\[^\s]+)`)

func parseWindowsMappings(out string) map[string]string {
	at := map[string]string{}
	for _, m := range netUseLine.FindAllStringSubmatch(out, -1) {
		at[strings.ToUpper(m[1])] = m[2]
	}
	return at
}

// Directory.GetLogicalDrives runs in the same operator logon as net use. Fail
// closed on unexpected output instead of allocating a possibly occupied letter.
func addWindowsDriveLetters(at map[string]string, out string) error {
	for _, line := range strings.Split(strings.TrimSpace(out), "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		if len(line) != 3 || line[0] < 'A' || line[0] > 'Z' || line[1:] != `:\` {
			return errors.New("unexpected logical drive enumeration output")
		}
		if at[line[:2]] == "" {
			at[line[:2]] = "local disk"
		}
	}
	return nil
}
