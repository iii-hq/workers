package main

import (
	"log"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

// observeFunction makes target, a function this worker registered, an
// observer of source. The observer stays pending until the source's worker
// registers it and accepts the observer. A refusal is logged: the worker keeps
// serving its own functions without that observer.
func observeFunction(client *iii.Client, source iii.Source, target string) error {
	handle, err := client.Observe(source, target)
	if err != nil {
		return err
	}
	handle.OnStateChange(func(state iii.ObserverState, reason *iii.ObserveReason) {
		if state == iii.ObserverError && reason != nil {
			log.Printf("%s does not observe %s: %s: %s", target, source.FunctionID, reason.Code, reason.Message)
		}
	})
	return nil
}
