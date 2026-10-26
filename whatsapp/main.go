package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"log"
	"os"
	"os/signal"
	"syscall"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
)

var version = "0.1.0"

const whatsmeowRevision = "1fbae988179a"

func main() {
	// Pairing codes, tokens and message bodies must never become SDK telemetry.
	_ = os.Setenv("III_DISABLE_TRACE_PAYLOADS", "1")
	log.SetFlags(log.Ldate | log.Ltime)
	if err := run(); err != nil {
		log.Print(err)
		os.Exit(1)
	}
}
func run() error {
	configPath := flag.String("config", "", "Optional YAML seed; existing central configuration remains authoritative")
	defaultURL := os.Getenv("III_URL")
	if defaultURL == "" {
		defaultURL = "ws://127.0.0.1:49134"
	}
	url := flag.String("url", defaultURL, "iii engine WebSocket URL")
	local := flag.Bool("local-config", false, "Use local configuration only")
	manifest := flag.Bool("manifest", false, "Print package information without connections or storage access")
	catalog := flag.Bool("catalog", false, "Print supported function schemas without connections or storage access")
	collect := flag.Bool("collect-interface", false, "Register interface metadata without backend calls, account storage, or business execution")
	showVersion := flag.Bool("version", false, "Print worker version")
	flag.Parse()
	if flag.NArg() != 0 {
		return errors.New("unexpected positional arguments")
	}
	if *manifest && *catalog {
		return errors.New("choose either --manifest or --catalog")
	}
	if *showVersion {
		fmt.Println("whatsapp", version)
		return nil
	}
	if *manifest {
		return printJSON(map[string]any{"name": "whatsapp", "version": version, "description": "Native WhatsApp and Meta Cloud API worker", "default_config": DefaultConfig(), "native_client": map[string]string{"repository": "https://github.com/tulir/whatsmeow", "revision": whatsmeowRevision}, "runtime": "standalone Go binary; no Node, browser or sandbox required", "trigger_types": []string{"whatsapp::events", "whatsapp::cloud::events"}, "console_ui": map[string]any{"scope": "whatsapp", "assets": []string{"whatsapp/page.js", "whatsapp/styles.css"}}})
	}
	if *catalog {
		return printJSON(NewRegistry(nil).Catalog())
	}
	seed := DefaultConfig()
	if *configPath != "" {
		b, err := os.ReadFile(*configPath)
		if err == nil {
			seed, err = parseConfigYAML(b)
		}
		if err != nil {
			if *local || *collect {
				return errors.New("unable to read valid local WhatsApp configuration")
			}
			log.Print("Optional configuration seed unavailable; using central configuration or defaults")
			seed = DefaultConfig()
		}
	}
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer cancel()
	client := iii.New(*url, iii.WithName("whatsapp"), iii.WithDescription("WhatsApp linked devices via whatsmeow and official Meta Cloud API"), iii.WithNamespace(Namespace()))
	defer client.Close()
	connectCtx, connectCancel := context.WithTimeout(ctx, 30*time.Second)
	err := client.Connect(connectCtx)
	connectCancel()
	if err != nil {
		return errors.New("unable to connect to iii engine")
	}
	config := seed
	if *local || *collect {
		config, err = runtimeConfig(seed)
	} else {
		config, err = ensureConfig(ctx, client, seed)
	}
	if err != nil {
		return err
	}
	if *collect {
		manager, err := newInterfaceManager(config)
		if err != nil {
			return err
		}
		if err := registerWorkerInterface(ctx, client, manager, *url, false); err != nil {
			return err
		}
		log.Print("WhatsApp interface registered; collection mode does not execute operations")
		<-ctx.Done()
		return nil
	}
	storage, err := newDatabaseStorage(ctx, client, config)
	if err != nil {
		return err
	}
	manager, err := NewManager(config, storage)
	if err != nil {
		return err
	}
	defer func() {
		shutdownCtx, shutdownCancel := context.WithTimeout(context.Background(), time.Duration(manager.Config().ShutdownTimeoutSecs)*time.Second)
		defer shutdownCancel()
		if err := manager.Shutdown(shutdownCtx); err != nil {
			log.Print("WhatsApp shutdown did not finish cleanly")
		}
	}()
	defer func() {
		if manager.tunnel == nil {
			return
		}
		shutdownCtx, shutdownCancel := context.WithTimeout(context.Background(), 15*time.Second)
		defer shutdownCancel()
		if manager.tunnel.Close(shutdownCtx) != nil {
			log.Print("Development tunnel shutdown did not finish cleanly; its lease will expire automatically")
		}
	}()
	if err = registerWorkerInterface(ctx, client, manager, *url, *local); err != nil {
		return err
	}
	log.Print("WhatsApp Go worker registered; sessions start only on explicit request")
	<-ctx.Done()
	return nil
}
func printJSON(value any) error {
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	return enc.Encode(value)
}
