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

package relay

import (
	"log"
	"net/http"
	"net/netip"
	"net/url"
	"strconv"

	"tailscale.com/derp/derpserver"
	"tailscale.com/types/key"
)

func New(verifyURL string) *derpserver.Server { return NewWithKey(key.NewNode(), verifyURL) }

// NewWithKey lets the hub retain its relay key across restarts. Invalid
// admission configuration uses an unsupported scheme, which fails locally.
func NewWithKey(k key.NodePrivate, verifyURL string) *derpserver.Server {
	parsed, err := url.Parse(verifyURL)
	valid := err == nil && parsed.Scheme == "http" && parsed.User == nil && parsed.Fragment == "" && parsed.RawQuery == ""
	if valid {
		addr, e := netip.ParseAddr(parsed.Hostname())
		port, pErr := strconv.Atoi(parsed.Port())
		valid = e == nil && addr.Unmap().IsLoopback() && pErr == nil && port > 0 && port <= 65535
	}
	if !valid {
		verifyURL = "deny-admission://invalid"
	}
	s := derpserver.New(k, func(f string, a ...any) { log.Printf("relay: "+f, a...) })
	s.SetVerifyClientURL(verifyURL)
	s.SetVerifyClientURLFailOpen(false)
	return s
}

func Handle(mux *http.ServeMux, s *derpserver.Server) {
	mux.Handle("/derp", derpserver.AddWebSocketSupport(s, derpserver.Handler(s)))
	mux.HandleFunc("/derp/probe", derpserver.ProbeHandler)
	mux.HandleFunc("/derp/latency-check", derpserver.ProbeHandler)
}
