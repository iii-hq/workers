package supervisor

import "errors"

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
func withPreparedCommand(prepare func() (func(), error), start func() error) error {
	close, err := prepare()
	if err != nil {
		return err
	}
	defer close()
	return start()
}
