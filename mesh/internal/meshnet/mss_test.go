package meshnet

import (
	"encoding/binary"
	"testing"

	"tailscale.com/net/packet"
	"tailscale.com/wgengine/filter"
)

func internetChecksum(b []byte) uint16 {
	var sum uint32
	for i := 0; i+1 < len(b); i += 2 {
		sum += uint32(binary.BigEndian.Uint16(b[i:]))
	}
	for sum>>16 > 0 {
		sum = sum&65535 + sum>>16
	}
	return ^uint16(sum)
}

func TestMSSClampPreservesTCPChecksum(t *testing.T) {
	for _, odd := range []bool{false, true} {
		pkt := make([]byte, 68)
		pkt[0] = 0x60
		pkt[6] = 6
		binary.BigEndian.PutUint16(pkt[4:], 28)
		tcp := pkt[40:]
		tcp[12] = 7 << 4
		tcp[13] = 2
		pos := 20
		if odd {
			tcp[pos] = 1
			pos++
		}
		copy(tcp[pos:], []byte{2, 4, 5, 180})
		pseudo := make([]byte, 40)
		pseudo[35] = 28
		pseudo[39] = 6
		binary.BigEndian.PutUint16(tcp[16:], internetChecksum(append(pseudo, tcp...)))
		var p packet.Parsed
		p.Decode(pkt)
		if got := clamped(nil)(&p, nil); got != filter.Accept {
			t.Fatal(got)
		}
		if mss := binary.BigEndian.Uint16(tcp[pos+2:]); mss != 1140 {
			t.Fatalf("MSS=%d", mss)
		}
		if internetChecksum(append(pseudo, tcp...)) != 0 {
			t.Fatal("invalid checksum after clamp")
		}
	}
}
