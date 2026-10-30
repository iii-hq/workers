package service

type windowsInstallation struct{ fetch, stop, place, register, start func() error }

func installWindows(i windowsInstallation) error {
	for _, step := range []func() error{i.fetch, i.stop, i.place, i.register, i.start} {
		if err := step(); err != nil {
			return err
		}
	}
	return nil
}

func uninstallWindows(resetRecovery, stop, remove func() error) error {
	// Disabling recovery before waiting prevents a failed service from racing leave.
	if err := resetRecovery(); err != nil {
		return err
	}
	if err := stop(); err != nil {
		return err
	}
	return remove()
}
