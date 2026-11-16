package client

import (
	"bytes"
	"cmp"
	"context"
	"encoding/json"
	"errors"
	"io"
	"log"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"sync"
	"time"

	iii "github.com/iii-hq/iii/sdk/packages/go/iii"
	"github.com/iii-hq/workers/mesh/internal/meshnet"
)

// directNet is what the peer to peer path uses of the machine's mesh stack (*meshnet.Mesh).
type directNet interface {
	SetPeerToPeer(on bool) error
	ListenPeers() (net.Listener, error)
	Dial(ctx context.Context, network, addr string) (net.Conn, error)
	WhoIs(from netip.AddrPort) (string, bool)
	Peer(name string) (netip.Addr, bool)
	Peers() []meshnet.PeerStatus
	HubHeard() time.Time
}

var _ directNet = (*meshnet.Mesh)(nil)

// shortcut: a fixed cap of 64 mesh calls in flight per calling machine, at the hub and at each direct target; make it a setting when a real mesh reaches it.
const MaxCallsInFlight = 64

// Eight more connections than calls, so the 65th call still gets mesh_busy.
const maxDirectConns = MaxCallsInFlight + 8

// Outlasts the target's 600 s local call, so the nearest timeout reports.
const directTimeout = 610 * time.Second

// Variables so tests can shorten them.
var (
	directGrace   = 10 * time.Minute // a removal reaches a machine only through its hub, so neither end uses the path longer without hearing it
	directHead    = 10 * time.Second // for the headers of a call to arrive at the target
	directIO      = 2 * time.Minute  // for its payload to arrive, and for its result to leave
	maxDirectBody = 16 << 20         // the documented limit of a mesh payload, each way
	refusalEvery  = 10 * time.Second // between two refusal log lines for one source address
	statusEvery   = 5 * time.Second  // from the status refresh a direct call asks for to the one more look after it
)

var errTooLarge = &iii.InvocationError{Code: "payload_too_large", Message: "mesh payloads are limited to 16 MiB"}

// The status a refusal travels with; the caller reads only "200 or not" and the body.
var directStatus = map[string]int{"FORBIDDEN": http.StatusForbidden, "mesh_busy": http.StatusTooManyRequests, "mesh_offline": http.StatusServiceUnavailable,
	"channels_unsupported": http.StatusBadRequest, "payload_too_large": http.StatusRequestEntityTooLarge}

// direct is one machine's peer to peer path: the listener that takes calls and the client that makes them.
type direct struct {
	b       *Bridge
	net     directNet
	version string
	l       net.Listener
	server  *http.Server
	client  *http.Client

	mu       sync.Mutex
	closed   bool
	calls    map[*directCall]struct{} // made by this machine, in flight
	conns    map[net.Conn]netip.Addr  // open on the listener, by source address
	inFlight map[netip.Addr]int       // running here, by source address: a rename keeps the address and would reset a count kept by name
	refused  map[netip.Addr]*refusals
	settling bool // the one more status look after a call is armed
}

type directCall struct {
	owner  string
	addr   netip.Addr
	cancel context.CancelFunc
}

type refusals struct {
	n    int       // since the last line
	next time.Time // when the next line may be written
}

// SetDirect opens the peer to peer path on n, or closes it when n is nil. It
// owns the order: a machine never asks to be shown with its port closed.
func (b *Bridge) SetDirect(n directNet, version string) error {
	b.mu.Lock()
	old := b.direct
	b.direct = nil
	b.changed()
	b.mu.Unlock()
	if old != nil {
		old.close() // local, so off never waits for the hub
		// shortcut: a withdrawal the backend refuses is not asked again and peers keep seeing this machine until its node restarts; keep the net and retry if EditPrefs ever fails in practice
		if err := old.net.SetPeerToPeer(false); err != nil {
			return err
		}
	}
	if n == nil {
		return nil
	}
	l, err := n.ListenPeers()
	if err != nil {
		return err
	}
	d := &direct{b: b, net: n, version: version, l: l, calls: map[*directCall]struct{}{}, conns: map[net.Conn]netip.Addr{}, inFlight: map[netip.Addr]int{}, refused: map[netip.Addr]*refusals{}}
	// No ReadTimeout or WriteTimeout: both count from the start of the request and would cut a 10 minute call; serveCall sets its own.
	d.server = &http.Server{Handler: http.HandlerFunc(d.serveCall), ReadHeaderTimeout: directHead, MaxHeaderBytes: 16 << 10, ConnState: d.connState}
	// One connection per call on both ends: these connections have no keepalive, so an idle one cannot be told from a dead one.
	d.server.SetKeepAlivesEnabled(false)
	d.client = &http.Client{
		// A fresh transport reads no proxy variables. The payload waits for the target's gate (see call).
		Transport: &http.Transport{DialContext: n.Dial, DisableKeepAlives: true, ExpectContinueTimeout: 10 * time.Second},
		// A target must not send this machine's payload elsewhere as this machine.
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
	}
	go d.server.Serve(l)
	if err = n.SetPeerToPeer(true); err != nil {
		d.close()
		return err
	}
	b.mu.Lock()
	b.direct = d
	b.changed()
	b.mu.Unlock()
	return nil
}

// MapChanged tells the peer to peer path that the machine's map may have lost a peer.
func (b *Bridge) MapChanged() {
	b.mu.Lock()
	d := b.direct
	b.mu.Unlock()
	if d != nil {
		d.prune()
	}
}

// close stops taking calls, ends the ones running here and cancels the ones this machine made.
func (d *direct) close() {
	d.mu.Lock()
	d.closed = true
	for c := range d.calls {
		c.cancel()
	}
	d.mu.Unlock()
	_ = d.l.Close() // here and not only in the server, which learns of the listener on another goroutine
	_ = d.server.Close()
}

// prune ends what a map change left without a peer: these connections have no keepalive, so nothing else would.
func (d *direct) prune() {
	d.mu.Lock()
	defer d.mu.Unlock()
	for c := range d.calls {
		if a, ok := d.net.Peer(c.owner); !ok || a != c.addr {
			c.cancel()
		}
	}
	for c, from := range d.conns {
		if _, ok := d.net.WhoIs(netip.AddrPortFrom(from, 0)); !ok {
			_ = c.Close()
		}
	}
}

// connState counts the open connections by source address, so one machine uses up only its own share.
func (d *direct) connState(c net.Conn, s http.ConnState) {
	d.mu.Lock()
	defer d.mu.Unlock()
	switch s {
	case http.StateNew:
		from, _ := netip.ParseAddrPort(c.RemoteAddr().String())
		held := 0
		for _, a := range d.conns {
			if a == from.Addr() {
				held++
			}
		}
		if held >= maxDirectConns {
			_ = c.Close()
			return
		}
		d.conns[c] = from.Addr()
	case http.StateClosed, http.StateHijacked:
		delete(d.conns, c)
	}
}

// serveCall is the gate of a direct call: who calls, what it may call and how
// much, all decided before one byte of the body is read.
func (d *direct) serveCall(w http.ResponseWriter, r *http.Request) {
	rc := http.NewResponseController(w)
	// Bounds the body and the server's drain of a refused one: once the headers are in, the server leaves both without a limit.
	if err := rc.SetReadDeadline(time.Now().Add(directIO)); err != nil {
		d.fail(w, err)
		return
	}
	from, _ := netip.ParseAddrPort(r.RemoteAddr)
	addr, id := from.Addr(), r.URL.Query().Get("function")
	name, known := d.net.WhoIs(from)
	d.b.mu.Lock()
	self := d.b.rt.Node
	_, shared := d.b.functions[id] // this machine's live share list, never the hub's catalog: no mirror, mesh:: or approval:: id is in it
	d.b.mu.Unlock()
	var refusal *iii.InvocationError
	switch {
	case r.Method != http.MethodPost || r.URL.Path != "/call" || !known || name == "hub" || name == self:
		refusal = &iii.InvocationError{Code: "FORBIDDEN", Message: "only another machine of this mesh may call"}
	case time.Since(d.net.HubHeard()) > directGrace:
		refusal = &iii.InvocationError{Code: "mesh_offline", Message: "the target machine has not heard its hub for 10 minutes"}
	case r.Header.Get("X-Mesh-Version") != d.version:
		refusal = &iii.InvocationError{Code: "FORBIDDEN", Message: "the target machine runs another mesh version"}
	case !shared:
		refusal = &iii.InvocationError{Code: "FORBIDDEN", Message: "function is not shared with this machine"}
	case r.ContentLength > int64(maxDirectBody):
		refusal = errTooLarge
	default:
		d.mu.Lock()
		if d.inFlight[addr] >= MaxCallsInFlight {
			refusal = &iii.InvocationError{Code: "mesh_busy", Message: "too many mesh calls in flight from this machine"}
		} else {
			d.inFlight[addr]++
		}
		d.mu.Unlock()
	}
	if refusal != nil {
		d.refuse(w, addr, name, id, refusal)
		return
	}
	defer func() {
		d.mu.Lock()
		if d.inFlight[addr]--; d.inFlight[addr] == 0 {
			delete(d.inFlight, addr)
		}
		d.mu.Unlock()
	}()
	body, err := io.ReadAll(http.MaxBytesReader(w, r.Body, int64(maxDirectBody)))
	if _, tooLarge := errors.AsType[*http.MaxBytesError](err); tooLarge {
		d.refuse(w, addr, name, id, errTooLarge)
		return
	} else if err != nil {
		// Answered, because a handler that returns without writing answers 200.
		d.fail(w, &iii.InvocationError{Code: "mesh_offline", Message: "the payload did not arrive in time"})
		return
	}
	// Left set on a call without a body, the deadline would fire in the server's watch of the connection and end the call as a lost one.
	_ = rc.SetReadDeadline(time.Time{})
	if len(body) == 0 {
		body = nil // the SDK sends nil as {} and an empty payload as invalid JSON
	}
	// The engine runs the function to its end whatever the caller does, so the slot waits for it too, as at the hub: a caller that hangs up must not free it.
	out, err := d.b.forward(id)(context.WithoutCancel(r.Context()), body)
	if err != nil {
		d.fail(w, err)
		return
	}
	raw, _ := out.(json.RawMessage)
	d.answer(w, http.StatusOK, raw)
}

// refuse answers e and logs at most one line per source address every 10 s: the caller sets the request rate, so it must not set the log rate.
func (d *direct) refuse(w http.ResponseWriter, from netip.Addr, name, id string, e *iii.InvocationError) {
	d.mu.Lock()
	r := d.refused[from]
	if r == nil {
		r = &refusals{}
		d.refused[from] = r
	}
	if now := time.Now(); now.Before(r.next) {
		r.n++
	} else {
		// Bounded and quoted, never the payload: the caller writes the id. The name is the hub's.
		log.Printf("mesh direct: refused %.200q for %s (busy %v), %d refusals since the last line", id, cmp.Or(name, from.String()), e.Code == "mesh_busy", r.n)
		r.n, r.next = 0, now.Add(refusalEvery)
	}
	d.mu.Unlock()
	d.fail(w, e)
}

// fail answers err as {"code","message"}; a plain error is invocation_failed, as the SDK reports it.
func (d *direct) fail(w http.ResponseWriter, err error) {
	e := &iii.InvocationError{Code: "invocation_failed", Message: err.Error()}
	errors.As(err, &e)
	body, _ := json.Marshal(map[string]string{"code": e.Code, "message": e.Message})
	d.answer(w, cmp.Or(directStatus[e.Code], http.StatusInternalServerError), body)
}

func (d *direct) answer(w http.ResponseWriter, status int, body []byte) {
	// A peer that does not read must not hold the call and its slot.
	_ = http.NewResponseController(w).SetWriteDeadline(time.Now().Add(directIO))
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_, _ = w.Write(body)
}

// call runs id on owner at addr over a connection of its own. Anything but an
// answer from the target is mesh_offline, and nothing is tried again or sent
// through the hub: the call may have run.
func (d *direct) call(ctx context.Context, owner string, addr netip.Addr, id string, data json.RawMessage) (any, error) {
	if time.Since(d.net.HubHeard()) > directGrace {
		// It cannot know the target is still in the mesh, so it sends it nothing.
		return nil, &iii.InvocationError{Code: "mesh_offline", Message: "this machine has not heard its hub for 10 minutes"}
	}
	if len(data) > maxDirectBody {
		return nil, errTooLarge
	}
	if HasChannelRefs(data) {
		return nil, errChannels // as the hub refuses them: a hostile target would not
	}
	offline := &iii.InvocationError{Code: "mesh_offline", Message: "no direct connection to " + owner}
	ctx, cancel := context.WithTimeout(ctx, directTimeout)
	defer cancel()
	c := &directCall{owner: owner, addr: addr, cancel: cancel}
	d.mu.Lock()
	closed := d.closed
	if !closed {
		d.calls[c] = struct{}{} // so prune and close can end it
	}
	d.mu.Unlock()
	if closed {
		return nil, offline
	}
	defer func() {
		d.mu.Lock()
		delete(d.calls, c)
		d.mu.Unlock()
	}()
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, "http://"+netip.AddrPortFrom(addr, meshnet.PeerPort).String()+"/call?function="+url.QueryEscape(id), bytes.NewReader(data))
	if err != nil {
		return nil, offline
	}
	req.Header.Set("X-Mesh-Version", d.version)
	// The payload goes only once the target's gate let the call in: a target that closes on an unread payload resets the connection, and the refusal is lost with it.
	req.Header.Set("Expect", "100-continue")
	res, err := d.client.Do(req)
	if err != nil {
		return nil, offline
	}
	defer res.Body.Close()
	body, err := io.ReadAll(io.LimitReader(res.Body, int64(maxDirectBody)+1))
	if err != nil {
		return nil, offline
	}
	if len(body) > maxDirectBody {
		return nil, errTooLarge
	}
	d.mu.Lock()
	tell := !d.settling // the page shows the path this call opened, whatever the call rate
	d.settling = true
	d.mu.Unlock()
	if tell {
		d.refresh()
		// A relayed path that turns direct has no event, nor has a call made meanwhile: look once more.
		time.AfterFunc(statusEvery, func() {
			d.mu.Lock()
			d.settling = false
			d.mu.Unlock()
			d.refresh()
		})
	}
	if res.StatusCode == http.StatusOK {
		if len(body) == 0 {
			return nil, nil
		}
		if HasChannelRefs(body) {
			return nil, errChannels
		}
		return json.RawMessage(body), nil
	}
	var e struct{ Code, Message string }
	if json.Unmarshal(body, &e) != nil || e.Code == "" {
		return nil, offline // not the target's answer, a redirect included
	}
	return nil, &iii.InvocationError{Code: e.Code, Message: e.Message}
}

// refresh asks for the status again, which reads each machine's path.
func (d *direct) refresh() {
	d.b.mu.Lock()
	d.b.changed()
	d.b.mu.Unlock()
}
