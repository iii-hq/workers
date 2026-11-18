package meshnet

import (
	"context"
	"tailscale.com/ipn"
)

func RegistrationErrorForTest(ctx context.Context, m *Mesh) string {
	var message string
	m.lb.WatchNotifications(ctx, ipn.NotifyInitialState, nil, func(n *ipn.Notify) bool {
		if n.ErrMessage != nil {
			message = *n.ErrMessage
			return false
		}
		return true
	})
	return message
}
