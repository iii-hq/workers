//go:build e2e

package e2e

import (
	"context"
	"errors"
	"fmt"
	"testing"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

func waitForPolicyRefusal(ctx context.Context, invoke func(context.Context) error) error {
	deadline, bounded := ctx.Deadline()
	if !bounded {
		return errors.New("policy refusal requires an absolute deadline")
	}
	for {
		if err := ctx.Err(); err != nil {
			return err
		}
		if !time.Now().Before(deadline) {
			return context.DeadlineExceeded
		}
		err := invoke(ctx)
		if canceled := ctx.Err(); canceled != nil {
			return canceled
		}
		if !time.Now().Before(deadline) {
			return context.DeadlineExceeded
		}
		var remote *iii.InvocationError
		if errors.As(err, &remote) && remote.Code == "FORBIDDEN" {
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(50 * time.Millisecond):
		}
	}
}

func TestPolicyRefusalDeadline(t *testing.T) {
	t.Run("before-deadline", func(t *testing.T) {
		ctx, cancel := context.WithTimeout(t.Context(), time.Second)
		defer cancel()
		err := waitForPolicyRefusal(ctx, func(callCtx context.Context) error {
			if got, _ := callCtx.Deadline(); got.IsZero() {
				t.Fatal("RPC lost deadline")
			}
			return &iii.InvocationError{Code: "FORBIDDEN", Message: "policy denied"}
		})
		if err != nil {
			t.Fatal(err)
		}
	})
	t.Run("late-policy-response", func(t *testing.T) {
		ctx, cancel := context.WithTimeout(t.Context(), 10*time.Millisecond)
		defer cancel()
		err := waitForPolicyRefusal(ctx, func(callCtx context.Context) error {
			<-callCtx.Done()
			return &iii.InvocationError{Code: "FORBIDDEN", Message: "policy denied"}
		})
		if !errors.Is(err, context.DeadlineExceeded) {
			t.Fatalf("late refusal accepted: %v", err)
		}
	})
	t.Run("cancellation-is-not-policy", func(t *testing.T) {
		ctx, cancel := context.WithTimeout(t.Context(), time.Second)
		defer cancel()
		err := waitForPolicyRefusal(ctx, func(context.Context) error {
			cancel()
			return fmt.Errorf("forbidden probe: %w", context.Canceled)
		})
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("cancellation accepted as refusal: %v", err)
		}
	})
	t.Run("message-is-not-policy-code", func(t *testing.T) {
		ctx, cancel := context.WithTimeout(t.Context(), 10*time.Millisecond)
		defer cancel()
		err := waitForPolicyRefusal(ctx, func(context.Context) error { return errors.New("forbidden transport error") })
		if !errors.Is(err, context.DeadlineExceeded) {
			t.Fatalf("transport error accepted as refusal: %v", err)
		}
	})
}
