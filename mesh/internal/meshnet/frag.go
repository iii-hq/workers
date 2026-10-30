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
	"math/rand/v2"
	"net/netip"
	"sync"
	"time"

	"github.com/tailscale/wireguard-go/tun"
)

const (
	// fragData is the most data one fragment of maxInner bytes carries: less
	// the IPv6 header and the fragment header, in the 8-byte units of the
	// offset.
	fragData = (maxInner - 40 - 8) &^ 7

	// A disco ping of probeSize bytes (WireGuard's 32 on a full 1280 packet)
	// is as long on the wire as a full mesh packet. Cut packets double a
	// Windows upload's packets and halve its speed, so they are cut only to a
	// peer that no such ping reached in probeWait. The answer is asked again
	// every probeEvery while long packets go to that peer.
	probeSize  = 1280 + 32
	probeWait  = 2 * time.Second
	probeEvery = 3 * time.Second
)

// fragmenter is the TUN with every IPv6 packet longer than maxInner, to a peer
// whose path may not carry a full packet, cut into fragments that are not
// (RFC 8200 4.5), as the engine reads it. The peer's OS puts them together
// again. TCP from Linux and macOS is held to maxInner by the MSS clamp; TCP
// from Windows, which sends segments of 1220 whatever the MSS, and large UDP
// and ICMP are not. A packet the OS fragmented already is cut finer, as the
// same datagram.
type fragmenter struct {
	tun.Device
	ping  func(netip.Addr) bool // whether a probeSize ping reaches the peer at that address
	mu    sync.Mutex
	paths map[netip.Addr]*path
	queue [][]byte // packets read and cut but not yet returned, in order
	mem   []byte   // queue's bytes
	id    uint32   // the last fragment identification
}

// path is what the last probe of a peer found.
type path struct {
	fits   bool // until the first probe answers, it does not
	at     time.Time
	asking bool
}

func newFragmenter(dev tun.Device, ping func(netip.Addr) bool) *fragmenter {
	return &fragmenter{Device: dev, ping: ping, paths: map[netip.Addr]*path{}, id: rand.Uint32()}
}

// tstun turns GRO off through these on Linux, where the TUN has them.
func (f *fragmenter) DisableUDPGRO() {
	if d, ok := f.Device.(tun.GRODevice); ok {
		d.DisableUDPGRO()
	}
}
func (f *fragmenter) DisableTCPGRO() {
	if d, ok := f.Device.(tun.GRODevice); ok {
		d.DisableTCPGRO()
	}
}

func (f *fragmenter) Read(bufs [][]byte, sizes []int, offset int) (int, error) {
	if len(f.queue) == 0 {
		n, err := f.Device.Read(bufs, sizes, offset)
		if err != nil || !f.cut(bufs[:n], sizes, offset) {
			return n, err
		}
	}
	n := 0
	for ; n < len(bufs) && len(f.queue) > 0; n++ {
		sizes[n] = copy(bufs[n][offset:], f.queue[0])
		f.queue = f.queue[1:]
	}
	return n, nil
}

// whole reports whether pkt goes as it is. A hop-by-hop or routing header
// must stay in front of a fragment header; the mesh carries neither, so a
// packet with one goes whole.
func (f *fragmenter) whole(pkt []byte) bool {
	return len(pkt) <= maxInner || pkt[0]>>4 != 6 || pkt[6] == 0 || pkt[6] == 43 || f.fits(netip.AddrFrom16([16]byte(pkt[24:40])))
}

// fits reports what the last probe of dst found, and probes again when that
// is older than probeEvery.
func (f *fragmenter) fits(dst netip.Addr) bool {
	f.mu.Lock()
	defer f.mu.Unlock()
	p := f.paths[dst]
	if p == nil {
		p = &path{}
		f.paths[dst] = p
	}
	if !p.asking && time.Since(p.at) > probeEvery {
		p.asking = true
		go func() {
			fits := f.ping(dst)
			f.mu.Lock()
			p.fits, p.at, p.asking = fits, time.Now(), false
			f.mu.Unlock()
		}()
	}
	return p.fits
}

// cut queues the packets in bufs, each cut into fragments unless it goes
// whole, and reports whether any was cut. If none was, it queues nothing.
func (f *fragmenter) cut(bufs [][]byte, sizes []int, offset int) bool {
	all := true
	for i, b := range bufs {
		all = all && f.whole(b[offset:offset+sizes[i]])
	}
	if all {
		return false
	}
	f.queue, f.mem = f.queue[:0], f.mem[:0]
	for i, b := range bufs {
		pkt := b[offset : offset+sizes[i]]
		if f.whole(pkt) {
			f.add(pkt)
			continue
		}
		// The fragment header: next header, reserved, offset in bytes (a
		// multiple of 8) with the more-fragments bit, identification.
		hdr := [8]byte{pkt[6]}
		off, more, data := uint16(0), false, pkt[40:]
		if pkt[6] == 44 {
			copy(hdr[:], pkt[40:48])
			off, more, data = binary.BigEndian.Uint16(pkt[42:])&^7, pkt[43]&1 == 1, pkt[48:]
		} else {
			f.id++
			binary.BigEndian.PutUint32(hdr[4:], f.id)
		}
		for len(data) > 0 {
			n := min(len(data), fragData)
			m := uint16(0)
			if n < len(data) || more {
				m = 1
			}
			binary.BigEndian.PutUint16(hdr[2:], off|m)
			p := f.add(pkt[:40], hdr[:], data[:n])
			binary.BigEndian.PutUint16(p[4:], uint16(8+n))
			p[6] = 44
			off, data = off+uint16(n), data[n:]
		}
	}
	return true
}

// add queues the concatenation of parts and returns it.
func (f *fragmenter) add(parts ...[]byte) []byte {
	start := len(f.mem)
	for _, p := range parts {
		f.mem = append(f.mem, p...)
	}
	p := f.mem[start:len(f.mem):len(f.mem)]
	f.queue = append(f.queue, p)
	return p
}
