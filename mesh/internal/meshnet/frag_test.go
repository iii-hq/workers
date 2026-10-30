package meshnet

import (
	"bytes"
	"encoding/binary"
	"net/netip"
	"testing"

	"github.com/tailscale/wireguard-go/tun"
)

type packetTUN struct {
	tun.Device
	packet []byte
}

func (d *packetTUN) Read(bufs [][]byte, sizes []int, offset int) (int, error) {
	sizes[0] = copy(bufs[0][offset:], d.packet)
	return 1, nil
}

func TestFragmenterPreservesDatagramAcrossReadBatches(t *testing.T) {
	original := make([]byte, 1280)
	original[0] = 0x60
	original[6] = 17
	binary.BigEndian.PutUint16(original[4:], 1240)
	copy(original[24:40], netip.MustParseAddr("fd7a::2").AsSlice())
	for i := 40; i < len(original); i++ {
		original[i] = byte(i)
	}
	f := newFragmenter(&packetTUN{packet: original}, func(netip.Addr) bool { return false })
	bufs := [][]byte{make([]byte, 1500)}
	sizes := make([]int, 1)
	var data []byte
	var id uint32
	for i := 0; i < 2; i++ {
		n, err := f.Read(bufs, sizes, 16)
		if err != nil || n != 1 {
			t.Fatalf("Read=%d %v", n, err)
		}
		pkt := bufs[0][16 : 16+sizes[0]]
		if len(pkt) > 1200 || pkt[6] != 44 || pkt[40] != 17 {
			t.Fatalf("invalid fragment header: %x", pkt[:48])
		}
		off := binary.BigEndian.Uint16(pkt[42:])
		if int(off&^7) != len(data) {
			t.Fatalf("fragment offset %d, data length %d", off, len(data))
		}
		if (off&1 != 0) != (i == 0) {
			t.Fatal("wrong more-fragments flag")
		}
		if i == 0 {
			id = binary.BigEndian.Uint32(pkt[44:])
		} else if binary.BigEndian.Uint32(pkt[44:]) != id {
			t.Fatal("datagram ID changed")
		}
		if int(binary.BigEndian.Uint16(pkt[4:])) != len(pkt)-40 {
			t.Fatal("wrong payload length")
		}
		data = append(data, pkt[48:]...)
	}
	if !bytes.Equal(data, original[40:]) {
		t.Fatal("reassembled datagram differs")
	}
}
