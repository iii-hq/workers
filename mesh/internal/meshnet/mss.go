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

package meshnet

import (
	"encoding/binary"
	"math/bits"

	"tailscale.com/net/packet"
	"tailscale.com/net/tstun"
	"tailscale.com/types/ipproto"
	"tailscale.com/wgengine/filter"
)

const (
	// A mesh packet crosses the underlay in WireGuard's 32 bytes, UDP's 8 and
	// IPv6's 40 (IPv4's 20). The TUN's MTU is 1280, IPv6's minimum, below
	// which no OS keeps IPv6 on it, so a full one is 1360 bytes on the wire.
	// A path of 1280 (a VPN, WSL2 behind a Tailscale adapter) must fragment
	// that, and one that drops fragments or ICMP drops it: small packets
	// pass, large ones vanish. A packet of at most this inside is at most 1280
	// outside; a longer one goes in fragments to a peer whose path may not
	// carry it (frag.go).
	maxInner = 1280 - 80

	// TCP, nearly all the mesh carries, is held to segments whose packet is
	// maxInner: less 40 of IPv6 and 20 of TCP. Windows ignores it.
	maxMSS = maxInner - 40 - 20
)

// clamped is next with the MSS of every TCP SYN lowered to maxMSS first. It
// runs on packets both to and from peers, so either end of a connection
// keeps both directions small, the hub's netstack node included.
func clamped(next tstun.FilterFunc) tstun.FilterFunc {
	return func(p *packet.Parsed, w *tstun.Wrapper) filter.Response {
		if p.IPProto == ipproto.TCP && p.TCPFlags&packet.TCPSyn != 0 {
			clampMSS(p.Transport())
		}
		if next == nil {
			return filter.Accept
		}
		return next(p, w)
	}
}

// clampMSS lowers the MSS option in TCP header tcp to maxMSS, in place, and
// updates the checksum for it (RFC 1624).
func clampMSS(tcp []byte) {
	if len(tcp) < 20 {
		return
	}
	end := min(int(tcp[12]>>4)*4, len(tcp))
	for i := 20; i+1 < end && tcp[i] != 0; {
		if tcp[i] == 1 { // no-op
			i++
			continue
		}
		if tcp[i] == 2 && tcp[i+1] == 4 && i+4 <= end {
			old := binary.BigEndian.Uint16(tcp[i+2:])
			if old <= maxMSS {
				return
			}
			binary.BigEndian.PutUint16(tcp[i+2:], maxMSS)
			o, n := old, uint16(maxMSS)
			if i%2 == 1 { // the checksum sums 16-bit words from the header's start
				o, n = bits.ReverseBytes16(o), bits.ReverseBytes16(n)
			}
			sum := uint32(^binary.BigEndian.Uint16(tcp[16:])) + uint32(^o) + uint32(n)
			sum = sum&0xffff + sum>>16
			sum = sum&0xffff + sum>>16
			binary.BigEndian.PutUint16(tcp[16:], ^uint16(sum))
			return
		}
		if tcp[i+1] < 2 {
			return
		}
		i += int(tcp[i+1])
	}
}
