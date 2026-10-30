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
	"context"
	"os"
	"path"
	"slices"
	"strings"
)

func (h *nativeHost) Mounts(context.Context) (map[string]string, error) {
	b, err := os.ReadFile("/proc/self/mountinfo")
	if err != nil {
		return nil, err
	}
	return parseMounts(string(b)), nil
}
func parseMounts(text string) map[string]string {
	type entry struct{ device, root, target, kind, source string }
	var entries []entry
	unescape := strings.NewReplacer(`\040`, " ", `\011`, "\t", `\012`, "\n", `\134`, `\`)
	for line := range strings.Lines(text) {
		f := strings.Fields(line)
		sep := slices.Index(f, "-")
		if len(f) < 6 || sep < 0 || sep+2 >= len(f) {
			continue
		}
		entries = append(entries, entry{f[2], unescape.Replace(f[3]), unescape.Replace(f[4]), f[sep+1], unescape.Replace(f[sep+2])})
	}
	at := map[string]string{}
	for _, e := range entries {
		source := e.source
		if !strings.HasPrefix(e.kind, "nfs") {
			// A bind source is relative to its filesystem, which may be mounted at /srv.
			source = e.root
			best := -1
			for _, base := range entries {
				if base.device != e.device || base.target == e.target || strings.HasPrefix(base.target, "/drives/") {
					continue
				}
				rel, ok := strings.CutPrefix(e.root, strings.TrimSuffix(base.root, "/")+"/")
				if e.root == base.root {
					rel = ""
					ok = true
				}
				if ok && (best < 0 || len(base.root) < best) {
					source = path.Join(base.target, rel)
					best = len(base.root)
				}
			}
		}
		at[e.target] = source
	}
	return at
}
