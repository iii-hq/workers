package supervisor

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"regexp"
	"sort"
	"strings"
	"sync"
	"time"
)

type Desired struct{ Name, Version, URL, SHA256 string }
type Result struct{ Name, Error string }
type Runtime struct{ Node, Namespace, URL string }
type Status struct {
	Name, Version, State, LogTail, Error string
	Restarts, PID                        int
}

type Supervisor struct {
	operations       sync.Mutex
	mu               sync.RWMutex
	cacheDir, logDir string
	operator         Operator
	runtime          Runtime
	workers          map[string]*worker
	closed           bool
	client           *http.Client
	lifetime         context.Context
	cancel           context.CancelFunc
}
type worker struct {
	desired Desired
	invalid error
	status  Status
	log     *rotatingLog
	cancel  context.CancelFunc
	done    chan struct{}
}

// New owns workers until Close, independently of individual request contexts.
// An empty namespace leaves workers suspended until authenticated SetRuntime.
func New(cacheDir, logDir string, op Operator, node, hubNS string) *Supervisor {
	rt := Runtime{Node: node, Namespace: hubNS}
	if node != "" && hubNS != "" {
		rt.URL = "ws://hub.iii.internal:49135"
	}
	lifetime, cancel := context.WithCancel(context.Background())
	return &Supervisor{lifetime: lifetime, cancel: cancel, cacheDir: cacheDir, logDir: logDir, operator: op, runtime: rt, workers: map[string]*worker{}, client: &http.Client{Timeout: 2 * time.Minute}}
}
func WorkerEnv(node, hubNS, worker string) []string {
	return []string{"III_URL=ws://hub.iii.internal:49135", "III_NAMESPACE=" + hubNS, "III_WORKER_NAME=" + worker, "III_CONFIG_NAME=" + node + "__" + worker}
}
func Backoff(restarts int) time.Duration {
	if restarts < 0 {
		restarts = 0
	}
	if restarts >= 6 {
		return time.Minute
	}
	return time.Second << restarts
}

var workerName = regexp.MustCompile(`^[a-z0-9-]+$`)
var nodeName = regexp.MustCompile(`^[a-z][a-z0-9-]{0,30}$`)

func validateRuntime(rt Runtime) error {
	if !nodeName.MatchString(rt.Node) || strings.TrimSpace(rt.Namespace) == "" || strings.TrimSpace(rt.Namespace) != rt.Namespace || strings.ContainsAny(rt.Namespace, "\x00\r\n") {
		return errors.New("worker runtime requires an assigned node and namespace")
	}
	if rt.URL != "" {
		u, err := url.Parse(rt.URL)
		if err != nil || (u.Scheme != "ws" && u.Scheme != "wss") || u.Hostname() == "" || u.User != nil || strings.ContainsAny(rt.URL, "\x00\r\n") {
			return errors.New("worker runtime requires a WebSocket URL")
		}
	}
	return nil
}

// SetRuntime records individual restart errors in Status so a failed old worker
// cannot prevent Node from subsequently applying a new complete desired set.
func (s *Supervisor) SetRuntime(ctx context.Context, rt Runtime) error {
	s.operations.Lock()
	defer s.operations.Unlock()
	if s.closed || s.lifetime.Err() != nil {
		return errors.New("worker supervisor is closed")
	}
	ctx, release := s.requestContext(ctx)
	defer release()
	if err := validateRuntime(rt); err != nil {
		return err
	}
	if s.runtime == rt {
		return nil
	}
	for _, w := range s.workers {
		s.stop(w)
	}
	s.runtime = rt
	for _, w := range s.workers {
		s.start(ctx, w)
	}
	return nil
}

// Apply replaces the complete desired set, including when it is empty.
func (s *Supervisor) Apply(ctx context.Context, ds []Desired) []Result {
	s.operations.Lock()
	defer s.operations.Unlock()
	ctx, release := s.requestContext(ctx)
	defer release()
	results := make([]Result, 0, len(ds))
	counts := make(map[string]int, len(ds))
	for _, d := range ds {
		counts[d.Name]++
	}
	for name, w := range s.workers {
		if counts[name] == 0 {
			s.stop(w)
			s.mu.Lock()
			delete(s.workers, name)
			s.mu.Unlock()
		}
	}
	for _, d := range ds {
		if s.closed {
			results = append(results, Result{Name: d.Name, Error: "worker supervisor is closed"})
			continue
		}
		d.SHA256 = strings.ToLower(d.SHA256)
		w := s.workers[d.Name]
		if w != nil && (w.desired != d || counts[d.Name] > 1) {
			s.stop(w)
			w = nil
		}
		if w == nil {
			w = &worker{desired: d, status: Status{Name: d.Name, Version: d.Version}}
			s.mu.Lock()
			s.workers[d.Name] = w
			s.mu.Unlock()
		}
		var err error
		if counts[d.Name] > 1 {
			w.invalid = errors.New("duplicate desired worker name")
			err = w.invalid
		} else {
			w.invalid = nil
			if w.cancel == nil {
				err = s.start(ctx, w)
			}
		}
		if err != nil {
			s.update(w, func(st *Status) { st.State = "error"; st.Error = err.Error(); st.PID = 0 })
		}
		s.mu.RLock()
		results = append(results, Result{Name: d.Name, Error: w.status.Error})
		s.mu.RUnlock()
	}
	return results
}
func (s *Supervisor) Status() []Status {
	s.mu.RLock()
	defer s.mu.RUnlock()
	result := make([]Status, 0, len(s.workers))
	for _, w := range s.workers {
		st := w.status
		if w.log != nil {
			st.LogTail = w.log.tail()
		}
		result = append(result, st)
	}
	sort.Slice(result, func(i, j int) bool { return result[i].Name < result[j].Name })
	return result
}
func (s *Supervisor) Close() error {
	s.cancel()
	s.operations.Lock()
	defer s.operations.Unlock()
	s.closed = true
	for _, w := range s.workers {
		s.stop(w)
	}
	s.mu.Lock()
	clear(s.workers)
	s.mu.Unlock()
	return nil
}
func (s *Supervisor) update(w *worker, f func(*Status)) {
	s.mu.Lock()
	defer s.mu.Unlock()
	f(&w.status)
}
func (s *Supervisor) stop(w *worker) {
	if w.cancel != nil {
		w.cancel()
		<-w.done
		w.cancel = nil
	}
	s.update(w, func(st *Status) { st.PID = 0 })
}
func (s *Supervisor) start(ctx context.Context, w *worker) (err error) {
	defer func() {
		if err != nil {
			s.update(w, func(st *Status) { st.State = "error"; st.Error = err.Error(); st.PID = 0 })
		}
	}()
	if w.invalid != nil {
		return w.invalid
	}
	if !workerName.MatchString(w.desired.Name) || w.desired.Version == "" {
		return errors.New("worker requires a safe name and version")
	}
	if len(s.runtime.Node)+2+len(w.desired.Name) > 64 {
		return errors.New("worker configuration ID exceeds 64 characters")
	}
	if err = validateArtifact(w.desired); err != nil {
		return err
	}
	if err = ctx.Err(); err != nil {
		return err
	}
	if s.runtime.URL == "" {
		s.update(w, func(st *Status) { st.State = "suspended"; st.Error = ""; st.PID = 0 })
		return nil
	}
	if err = validateRuntime(s.runtime); err != nil {
		return err
	}
	if err = validateOperator(s.operator); err != nil {
		return err
	}
	path, err := s.artifact(ctx, w.desired)
	if err != nil {
		return err
	}
	binary, err := executable(path, w.desired.Name)
	if err != nil {
		return err
	}
	log, err := openLog(s.logDir, w.desired.Name)
	if err != nil {
		return err
	}
	cmd, err := workerCommand(binary, w.desired.Name, s.operator, s.runtime, log)
	if err == nil {
		err = ctx.Err()
	}
	if err == nil {
		err = startOperator(cmd, s.operator)
	}
	if err != nil {
		log.Close()
		return fmt.Errorf("start %s: %w", w.desired.Name, err)
	}
	runCtx, cancel := context.WithCancel(context.Background())
	w.cancel = cancel
	w.done = make(chan struct{})
	s.mu.Lock()
	w.log = log
	w.status.State = "running"
	w.status.Error = ""
	w.status.PID = cmd.Process.Pid
	w.status.Restarts = 0
	s.mu.Unlock()
	go s.monitor(runCtx, w, path, cmd, log, s.runtime)
	return nil
}

func (s *Supervisor) requestContext(ctx context.Context) (context.Context, func()) {
	request, cancel := context.WithCancel(ctx)
	stop := context.AfterFunc(s.lifetime, cancel)
	if s.lifetime.Err() != nil {
		cancel()
	}
	return request, func() { stop(); cancel() }
}
