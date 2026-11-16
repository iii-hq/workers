package client

import (
	"context"
	"log"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

// Observe keeps an observer for as long as c lives: the engine disposes the observers of a connection that stays away past its grace period, and the SDK does not send a disposed one again.
// resumed runs once the new observer is active: the events since the disposal reached no one. ctx ends the wait for that, and must end with c.
func Observe(ctx context.Context, c *iii.Client, source iii.Source, target string, opts iii.ObserverOptions, resumed func()) error {
	return keep(ctx, c, source, target, opts, resumed, false)
}

func keep(ctx context.Context, c *iii.Client, source iii.Source, target string, opts iii.ObserverOptions, resumed func(), again bool) error {
	h, err := c.Observe(source, target, opts)
	if err != nil {
		return err
	}
	h.OnStateChange(func(s iii.ObserverState, _ *iii.ObserveReason) {
		if s != iii.ObserverDisposed {
			return
		}
		opts.ID = "" // a new id: an engine that still remembers the disposed one would answer disposed again, in a loop
		if err := keep(ctx, c, source, target, opts, resumed, true); err != nil {
			log.Printf("mesh: observe %s again: %v", source.FunctionID, err)
		}
	})
	go func() {
		err := h.Ready(ctx)
		switch {
		case err == nil && again:
			resumed()
		case err != nil && h.State() == iii.ObserverError:
			log.Printf("mesh: observe %s: %v", source.FunctionID, err) // a refusal is final, and no event reaches this observer
		}
	}()
	return nil
}
