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

package service

import (
	"archive/zip"
	"bytes"
	"context"
	"crypto/sha256"
	"fmt"
	"io"
	"net/http"
	"runtime"
	"time"
)

const wintunURL = "https://www.wintun.net/builds/wintun-0.14.1.zip"
const wintunArchiveSHA256 = "07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51"

// DLL digests are computed from the archive pinned by the official download page.
const wintunAMD64SHA256 = "e5da8447dc2c320edc0fc52fa01885c103de8c118481f683643cacc3220dafce"
const wintunARM64SHA256 = "f7ba89005544be9d85231a9e0d5f23b2d15b3311667e2dad0debd344918a3f80"

func verifyDigest(b []byte, expected string) error {
	if fmt.Sprintf("%x", sha256.Sum256(b)) != expected {
		return fmt.Errorf("Wintun SHA256 mismatch: expected %s", expected)
	}
	return nil
}
func verifyWintunArchive(b []byte) error { return verifyDigest(b, wintunArchiveSHA256) }
func verifyWintun(b []byte) error        { return verifyWintunArchitecture(b, runtime.GOARCH) }
func verifyWintunArchitecture(b []byte, arch string) error {
	switch arch {
	case "amd64":
		return verifyDigest(b, wintunAMD64SHA256)
	case "arm64":
		return verifyDigest(b, wintunARM64SHA256)
	default:
		return fmt.Errorf("unsupported Wintun architecture %s", arch)
	}
}
func downloadWintun(ctx context.Context) ([]byte, []byte, error) {
	client := &http.Client{Timeout: time.Minute, CheckRedirect: func(r *http.Request, via []*http.Request) error {
		if r.URL.Scheme != "https" || r.URL.Hostname() != "www.wintun.net" || len(via) > 5 {
			return fmt.Errorf("unsafe Wintun redirect")
		}
		return nil
	}}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, wintunURL, nil)
	if err != nil {
		return nil, nil, err
	}
	res, err := client.Do(req)
	if err != nil {
		return nil, nil, err
	}
	defer res.Body.Close()
	if res.StatusCode != http.StatusOK {
		return nil, nil, fmt.Errorf("Wintun download: HTTP %s", res.Status)
	}
	b, err := io.ReadAll(io.LimitReader(res.Body, 4<<20))
	if err != nil {
		return nil, nil, err
	}
	if err = verifyWintunArchive(b); err != nil {
		return nil, nil, err
	}
	z, err := zip.NewReader(bytes.NewReader(b), int64(len(b)))
	if err != nil {
		return nil, nil, err
	}
	read := func(name string) ([]byte, error) {
		f, err := z.Open(name)
		if err != nil {
			return nil, err
		}
		defer f.Close()
		return io.ReadAll(io.LimitReader(f, 4<<20))
	}
	dll, err := read("wintun/bin/" + runtime.GOARCH + "/wintun.dll")
	if err != nil {
		return nil, nil, err
	}
	if err = verifyWintun(dll); err != nil {
		return nil, nil, err
	}
	license, err := read("wintun/LICENSE.txt")
	return dll, license, err
}
