package cli

import (
	"context"
	"errors"
	"fmt"
	"net/url"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/iii-hq/workers/mesh/internal/elevate"
	"github.com/iii-hq/workers/mesh/internal/join"
)

type JoinArgs struct {
	PublicURL, Code string
	Port            int
}

func ParseJoin(args []string) (JoinArgs, error) {
	a := JoinArgs{Port: 41643}
	var positional []string
	for i := 0; i < len(args); i++ {
		arg := args[i]
		if arg == "--port" {
			i++
			if i >= len(args) {
				return a, errors.New("--port requires a value")
			}
			arg = "--port=" + args[i]
		}
		if value, ok := strings.CutPrefix(arg, "--port="); ok {
			p, err := strconv.Atoi(value)
			if err != nil || p < 0 || p > 65535 {
				return a, errors.New("port must be between 0 and 65535")
			}
			a.Port = p
		} else if strings.HasPrefix(arg, "-") {
			return a, fmt.Errorf("unknown join option %s", arg)
		} else {
			positional = append(positional, arg)
		}
	}
	if len(positional) != 2 {
		return a, errors.New("usage: mesh join [--port 41643] <url> <code>")
	}
	a.PublicURL, a.Code = positional[0], positional[1]
	if err := validateURL(a.PublicURL); err != nil {
		return a, err
	}
	if _, err := join.ParseCode(a.Code); err != nil {
		return a, err
	}
	return a, nil
}
func validateURL(raw string) error {
	u, err := url.Parse(raw)
	if err != nil || (u.Scheme != "https" && u.Scheme != "http") || u.Hostname() == "" || u.User != nil || u.RawQuery != "" || u.ForceQuery || strings.Contains(raw, "#") {
		return errors.New("invalid hub public URL")
	}
	return nil
}

func Join(ctx context.Context, publicURL, code string, port int) error {
	if !elevate.Is() {
		return errors.New("mesh join requires elevation")
	}
	return defaults("").join(ctx, publicURL, code, port)
}
func (a *app) join(ctx context.Context, publicURL, code string, port int) error {
	if _, err := ParseJoin([]string{publicURL, code, "--port=" + strconv.Itoa(port)}); err != nil {
		return err
	}
	if exists(filepath.Join(a.paths.State, "join.json")) || exists(filepath.Join(a.paths.State, "removed")) {
		return errors.New("this machine is already installed; run mesh leave before joining again")
	}
	operator, err := a.operator()
	if err != nil {
		return err
	}
	if err = a.checkOperator(operator); err != nil {
		return err
	}
	hostname, err := a.hostname()
	if err != nil {
		return err
	}
	secret, pin, err := a.redeem(ctx, publicURL, code)
	if err != nil {
		return err
	}
	identity, err := a.identity(ctx, publicURL)
	if err != nil {
		return err
	}
	if err = identity.Verify(pin); err != nil {
		return err
	}
	s := State{PublicURL: publicURL, Secret: secret, Identity: identity, Operator: operator, RequestedName: requestedName(hostname), Port: port}
	if a.resolveOperator != nil {
		s.Operator, s.OperatorID, err = a.resolveOperator(operator)
		if err != nil {
			return err
		}
	}
	if err = saveState(a.paths.State, s); err != nil {
		return err
	}
	binary, err := a.executable()
	if err != nil {
		return err
	}
	if err = a.install(binary, s.operatorAccount()); err != nil {
		return fmt.Errorf("saved enrollment but service installation failed (run mesh leave before retrying): %w", err)
	}
	return nil
}
func requestedName(hostname string) string {
	hostname = strings.ToLower(strings.SplitN(hostname, ".", 2)[0])
	var b strings.Builder
	for _, c := range hostname {
		if c >= 'a' && c <= 'z' || c >= '0' && c <= '9' || c == '-' {
			b.WriteRune(c)
		}
	}
	name := b.String()
	if name == "" || name[0] < 'a' || name[0] > 'z' {
		name = "node-" + name
	}
	if len(name) > 31 {
		name = name[:31]
	}
	switch name {
	case "mesh", "hub", "engine", "iii", "configuration", "harness", "shell", "coder":
		return "node-" + name
	}
	return name
}
