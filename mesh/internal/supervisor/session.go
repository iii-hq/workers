package supervisor

import (
	"errors"
	"os/exec"
	"sync"
)

type operatorSession struct {
	id     uint32
	sid    string
	active bool
}

func selectOperatorSession(sid string, sessions []operatorSession) (uint32, error) {
	if sid == "" || sid == "S-1-5-18" || sid == "S-1-5-19" || sid == "S-1-5-20" {
		return 0, errors.New("workers require an interactive operator")
	}
	for _, active := range []bool{true, false} {
		for _, s := range sessions {
			if s.id != 0 && s.active == active && s.sid == sid {
				return s.id, nil
			}
		}
	}
	return 0, errors.New("original operator is not logged in; refusing another user or SYSTEM token")
}

func validateSessionToken(savedSID string, selectedSession uint32, sid string, session uint32, elevated bool) error {
	if savedSID == "" || selectedSession == 0 || savedSID != sid || session != selectedSession || elevated {
		return errors.New("operator token identity, session or elevation mismatch")
	}
	return nil
}

// The parent factory atomically creates a suspended, handle-free process in
// the authenticated session and a kill-on-close job. Its returned closures own
// the parent and job separately; factory failures must release partial state.
func startWithSessionParent(cmd *exec.Cmd, prepare func() (func(), error), parent func() (release func(), stop func(), err error), start func(*exec.Cmd) error) (func(), error) {
	closeToken, err := prepare()
	if err != nil {
		return nil, err
	}
	defer closeToken()
	closeParent, stop, err := parent()
	if err != nil {
		return nil, err
	}
	defer closeParent()
	if err := start(cmd); err != nil {
		stop()
		return nil, err
	}
	return sync.OnceFunc(stop), nil
}
func withPreparedCommand(prepare func() (func(), error), start func() error) error {
	close, err := prepare()
	if err != nil {
		return err
	}
	defer close()
	return start()
}
