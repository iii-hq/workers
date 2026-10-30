package main

import (
	"context"
	"flag"
	"log"
	"os"
	"os/signal"
	"syscall"

	"github.com/iii-hq/workers/mesh/internal/hub"
)

var version string

func main() {
	defaultURL := os.Getenv("III_URL")
	if defaultURL == "" {
		defaultURL = "ws://127.0.0.1:49134"
	}
	url := flag.String("url", defaultURL, "iii engine WebSocket URL")
	local := flag.Bool("local-config", false, "Register the interface using defaults without backend initialization")
	collect := flag.Bool("collect-interface", false, "Register interface metadata without backend initialization")
	flag.Parse()
	if flag.NArg() != 0 {
		log.Fatal("unexpected positional arguments")
	}
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	if err := hub.Run(ctx, hub.Options{URL: *url, Version: version, LocalConfig: *local, CollectInterface: *collect}); err != nil {
		log.Fatal(err)
	}
}
