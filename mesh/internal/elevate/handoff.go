package elevate

import "errors"

func validateWindowsHandoff(sid string, parentSession, childSession uint32, sameExecutable bool) error {
	if sid == "" || sid == "S-1-5-18" || sid == "S-1-5-19" || sid == "S-1-5-20" || parentSession == 0 || parentSession != childSession || !sameExecutable {
		return errors.New("cannot authenticate original Windows operator across elevation")
	}
	return nil
}
