package coordinator

import (
	"crypto/ed25519"
	"crypto/subtle"
	"errors"
	"slices"
	"time"

	"github.com/iii-hq/workers/mesh/internal/join"
)

func (c *Coordinator) IssueToken(publicURL string, opts join.TokenOptions) (id, raw string, err error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.ctx.Err() != nil {
		return "", "", errors.New("coordinator closed")
	}
	token, raw, err := join.NewToken(publicURL, c.st.Identity.Public().(ed25519.PublicKey), opts)
	if err != nil {
		return "", "", err
	}
	if opts.ExpiresInSecs > 0 {
		token.Expires = c.clock.Now().Add(time.Duration(opts.ExpiresInSecs) * time.Second)
	}
	before := c.st.Tokens
	c.st.Tokens = append(slices.Clone(before), token)
	if err = c.save(); err != nil {
		c.st.Tokens = before
		return "", "", err
	}
	return token.ID, raw, nil
}

// RedeemToken is a preflight only. Registration commits consumption with its node.
func (c *Coordinator) RedeemToken(secret string) (join.TokenGrant, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	i, err := c.token(secret)
	if err != nil {
		return join.TokenGrant{}, err
	}
	t := c.st.Tokens[i]
	return join.TokenGrant{ID: t.ID, Ephemeral: t.Ephemeral}, nil
}
func (c *Coordinator) token(secret string) (int, error) {
	hash := join.TokenHash(secret)
	for i, t := range c.st.Tokens {
		if subtle.ConstantTimeCompare([]byte(hash), []byte(t.Hash)) == 1 {
			if t.Revoked || !t.Expires.IsZero() && !c.clock.Now().Before(t.Expires) || !t.Reusable && len(t.Nodes) > 0 {
				return -1, join.ErrToken
			}
			return i, nil
		}
	}
	return -1, join.ErrToken
}
func (c *Coordinator) RevokeToken(id string) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	before := c.st.Tokens
	if !c.revokeToken(id) {
		return errors.New("unknown join token")
	}
	if err := c.save(); err != nil {
		c.st.Tokens = before
		return err
	}
	return nil
}
func (c *Coordinator) RevokeForNode(name string) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	n := c.byName(name)
	if n == nil {
		return errors.New("unknown node")
	}
	if n.TokenID == "" {
		return nil
	}
	before := c.st.Tokens
	c.revokeToken(n.TokenID)
	if err := c.save(); err != nil {
		c.st.Tokens = before
		return err
	}
	return nil
}
func (c *Coordinator) revokeToken(id string) bool {
	for i, t := range c.st.Tokens {
		if t.ID == id {
			c.st.Tokens = slices.Clone(c.st.Tokens)
			c.st.Tokens[i].Revoked = true
			return true
		}
	}
	return false
}

func (c *Coordinator) SetJoinLimit(limit *join.Limit) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.limit = limit
}
