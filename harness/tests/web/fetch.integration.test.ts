/**
 * End-to-end transport tests for executeFetch against a real loopback
 * `http.createServer`. Unlike fetch.test.ts (which only exercises the guard
 * surface + helpers), these drive the node:http request path the worker now
 * uses — proving IP pinning, per-hop SSRF re-validation, the byte cap, the
 * timeout, and POST body handling actually work over a socket.
 *
 * Plaintext loopback only: the HTTPS servername/cert-identity path can't be
 * faithfully unit-tested without generating a trusted cert (own mini-project),
 * so it stays correct-by-construction (options.servername = hostname) plus the
 * live smoke step. allow_loopback defaults true, so 127.0.0.1 targets pass the
 * SSRF guard and dial the test server.
 */

import type { AddressInfo } from 'node:net';
import { type Server, createServer } from 'node:http';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { loadWebConfig } from '../../src/web/config.js';
import { executeFetch } from '../../src/web/fetch.js';

let server: Server;
let base: string;
let lastReceivedHost: string | undefined;

beforeAll(async () => {
  server = createServer((req, res) => {
    lastReceivedHost = req.headers.host;
    const url = req.url ?? '/';
    if (url === '/ok') {
      res.writeHead(200, { 'content-type': 'text/plain', 'x-custom': 'yes' });
      res.end('hello');
      return;
    }
    if (url === '/big') {
      res.writeHead(200);
      res.end(Buffer.alloc(10_000, 0x41));
      return;
    }
    if (url === '/redirect-to-blocked') {
      res.writeHead(302, { location: 'http://169.254.169.254/latest/meta-data/' });
      res.end();
      return;
    }
    if (url === '/redirect-relative') {
      res.writeHead(302, { location: '/ok' });
      res.end();
      return;
    }
    if (url === '/slow') {
      setTimeout(() => {
        res.writeHead(200);
        res.end('late');
      }, 300);
      return;
    }
    if (url === '/echo' && req.method === 'POST') {
      const chunks: Buffer[] = [];
      req.on('data', (c) => chunks.push(Buffer.from(c)));
      req.on('end', () => {
        res.writeHead(200, { 'content-type': 'application/json' });
        res.end(
          JSON.stringify({
            received_content_type: req.headers['content-type'] ?? null,
            body: Buffer.concat(chunks).toString('utf8'),
          }),
        );
      });
      return;
    }
    res.writeHead(404);
    res.end('nope');
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const port = (server.address() as AddressInfo).port;
  base = `http://127.0.0.1:${port}`;
});

afterAll(async () => {
  await new Promise<void>((resolve) => server.close(() => resolve()));
});

const cfg = () => loadWebConfig({});

describe('executeFetch transport (loopback http server)', () => {
  it('GETs a body, status, and headers over a real socket', async () => {
    const r = await executeFetch({ url: `${base}/ok` }, cfg());
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.status).toBe(200);
      expect(r.body).toBe('hello');
      expect(r.headers['x-custom']).toBe('yes');
      expect(r.bytes_truncated).toBe(false);
    }
    // Host header is derived from the URL authority (pinning doesn't clobber it).
    expect(lastReceivedHost).toBe(base.replace('http://', ''));
  });

  it('truncates a response that exceeds max_bytes', async () => {
    const r = await executeFetch({ url: `${base}/big`, max_bytes: 100 }, cfg());
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.bytes_truncated).toBe(true);
      expect(r.body.length).toBe(100);
    }
  });

  it('re-runs the SSRF check on each redirect hop (302 → metadata is blocked)', async () => {
    const r = await executeFetch({ url: `${base}/redirect-to-blocked` }, cfg());
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.error).toBe('blocked_host');
      expect(r.message).toMatch(/169\.254\.169\.254/);
    }
  });

  it('follows a same-host relative redirect and records the chain', async () => {
    const r = await executeFetch({ url: `${base}/redirect-relative` }, cfg());
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.body).toBe('hello');
      expect(r.redirect_chain).toEqual([`${base}/redirect-relative`]);
    }
  });

  it('returns error:timeout when the server is slower than timeout_ms', async () => {
    const r = await executeFetch({ url: `${base}/slow`, timeout_ms: 50 }, cfg());
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('timeout');
  });

  it('sends a JSON body with content-type and parses the response', async () => {
    const r = await executeFetch(
      { url: `${base}/echo`, method: 'POST', json: { a: 1 }, response_format: 'json' },
      cfg(),
    );
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.json).toMatchObject({
        received_content_type: 'application/json',
        body: '{"a":1}',
      });
    }
  });

  it('returns transport_error when the connection is refused', async () => {
    // Port 1 on loopback is closed; SSRF guard passes (loopback allowed),
    // the socket connect fails → transport_error (not a thrown exception).
    const r = await executeFetch({ url: 'http://127.0.0.1:1/' }, cfg());
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('transport_error');
  });
});
