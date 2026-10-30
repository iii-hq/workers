package main

import (
	"context"
	"errors"
	"flag"
	"io"
	"log"
	"os"
	"os/signal"
	"syscall"

	"github.com/iii-hq/workers/mesh/internal/cli"
	"github.com/iii-hq/workers/mesh/internal/hub"
	"github.com/iii-hq/workers/mesh/internal/node"
)

var version string

func main() {
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	if err := run(ctx, os.Args[1:], os.Stdout); err != nil && err != node.ErrRestartRequired {
		log.Fatal(err)
	}
}
func run(ctx context.Context, args []string, w io.Writer) error {
	if len(args) > 0 {
		switch args[0] {
		case "join", "status", "leave", "node":
			return cli.Execute(ctx, args, version, w)
		}
	}
	flags := flag.NewFlagSet("mesh", flag.ContinueOnError)

	defaultURL := os.Getenv("III_URL")
	if defaultURL == "" {
		defaultURL = "ws://127.0.0.1:49134"
	}
	url := flags.String("url", defaultURL, "iii engine WebSocket URL")
	local := flags.Bool("local-config", false, "Register the interface using defaults without backend initialization")
	collect := flags.Bool("collect-interface", false, "Register interface metadata without backend initialization")
	if err := flags.Parse(args); err != nil {
		return err
	}
	if flags.NArg() != 0 {
		return errors.New("unexpected positional arguments")
	}
	return hub.Run(ctx, hub.Options{URL: *url, Version: version, LocalConfig: *local, CollectInterface: *collect})
}
