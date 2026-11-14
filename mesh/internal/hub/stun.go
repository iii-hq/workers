package hub

import (
	"net"

	"tailscale.com/net/stun"
)

func listenSTUN(port int) (*net.UDPConn, error) {
	c, err := net.ListenUDP("udp", &net.UDPAddr{Port: port})
	if err != nil {
		return nil, err
	}
	go func() {
		var b [1500]byte
		for {
			n, addr, err := c.ReadFromUDPAddrPort(b[:])
			if err != nil {
				return
			}
			id, err := stun.ParseBindingRequest(b[:n])
			if err == nil {
				_, _ = c.WriteToUDPAddrPort(stun.Response(id, addr), addr)
			}
		}
	}()
	return c, nil
}
