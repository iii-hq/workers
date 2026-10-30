package meshnet

import "tailscale.com/tailcfg"

const gatewayCapability tailcfg.NodeCapability = "https://iii.dev/mesh/gateway"

// GatewayPort is present only after the authenticated control map advertises
// the hub service. Callers must wait rather than assume the default port.
func (m *Mesh) GatewayPort() (uint16, bool) { p := m.gateway.Load(); return uint16(p), p != 0 }

// GatewayChanges coalesces updates; consumers re-read GatewayPort and reconnect
// their worker bus sessions when it changes. It remains open until Mesh closes.
func (m *Mesh) GatewayChanges() <-chan struct{} { return m.gatewayChanges }

func (m *Mesh) updateGateway() {
	var port uint32
	if nm := m.lb.NetMapWithPeers(); nm != nil {
		for _, p := range nm.Peers {
			if p.Name() != "hub" {
				continue
			}
			values, err := tailcfg.UnmarshalNodeCapJSON[struct {
				Port int `json:"gateway_port"`
			}](p.CapMap().AsMap(), gatewayCapability)
			if err == nil && len(values) == 1 && values[0].Port > 0 && values[0].Port <= 65535 {
				port = uint32(values[0].Port)
			}
		}
	}
	if old := m.gateway.Swap(port); old != port {
		select {
		case m.gatewayChanges <- struct{}{}:
		default:
		}
	}
}
