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

import { Buffer } from 'node:buffer';
import type { AddressInfo } from 'node:net';
import { type IncomingHttpHeaders, type Server, createServer } from 'node:http';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { loadWebConfig } from '../../src/web/config.js';
import { BROWSER_USER_AGENT } from '../../src/web/convert.js';
import { executeFetch } from '../../src/web/fetch.js';
import type { FetchImageResult, FetchResult } from '../../src/web/schemas.js';

// 1x1 transparent PNG.
const PNG_BYTES = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==',
  'base64',
);

function isImageEnvelope(r: FetchResult | FetchImageResult): r is FetchImageResult {
  return 'content' in r;
}

function asFetchResult(r: FetchResult | FetchImageResult): FetchResult {
  if (isImageEnvelope(r)) throw new Error('expected a FetchResult envelope, got an image envelope');
  return r;
}

let server: Server;
let base: string;
let lastReceivedHost: string | undefined;
let lastReceivedHeaders: IncomingHttpHeaders = {};
let cfChallengeHits = 0;

beforeAll(async () => {
  server = createServer((req, res) => {
    lastReceivedHost = req.headers.host;
    lastReceivedHeaders = req.headers;
    const url = req.url ?? '/';
    if (url === '/html') {
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' });
      res.end(
        '<html><head><script>var hidden = 1;</script></head><body><h1>Title</h1><p>para</p></body></html>',
      );
      return;
    }
    if (url === '/img.png') {
      res.writeHead(200, { 'content-type': 'image/png' });
      res.end(PNG_BYTES);
      return;
    }
    if (url === '/img-404') {
      res.writeHead(404, { 'content-type': 'image/png' });
      res.end(PNG_BYTES);
      return;
    }
    if (url === '/img-svg') {
      res.writeHead(200, { 'content-type': 'image/svg+xml' });
      res.end('<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>');
      return;
    }
    if (url === '/img-empty') {
      res.writeHead(200, { 'content-type': 'image/png' });
      res.end();
      return;
    }
    if (url === '/deep-html') {
      // ~5000 nested divs (~55 KB): overflows Turndown's recursion well
      // below the byte cap — exercises the transform-failure fallback.
      const depth = 5000;
      res.writeHead(200, { 'content-type': 'text/html' });
      res.end(`${'<div>'.repeat(depth)}core${'</div>'.repeat(depth)}`);
      return;
    }
    if (url === '/cf-challenge') {
      // First request with a browser UA gets the Cloudflare challenge;
      // the honest-UA retry succeeds.
      if ((req.headers['user-agent'] ?? '') === BROWSER_USER_AGENT) {
        cfChallengeHits++;
        res.writeHead(403, { 'cf-mitigated': 'challenge' });
        res.end('blocked');
        return;
      }
      res.writeHead(200, { 'content-type': 'text/html' });
      res.end('<p>welcome human</p>');
      return;
    }
    if (url === '/cf-always') {
      // Challenges EVERY user agent — the honest-UA retry also fails.
      cfChallengeHits++;
      res.writeHead(403, { 'cf-mitigated': 'challenge' });
      res.end('blocked');
      return;
    }
    if (url === '/cf-managed') {
      // 403 with a cf-mitigated value that is NOT 'challenge' — no retry.
      cfChallengeHits++;
      res.writeHead(403, { 'cf-mitigated': 'managed' });
      res.end('blocked');
      return;
    }
    if (url === '/xhtml') {
      res.writeHead(200, { 'content-type': 'application/xhtml+xml' });
      res.end('<html><body><h1>XTitle</h1></body></html>');
      return;
    }
    if (url === '/redirect-to-img') {
      res.writeHead(302, { location: '/img.png' });
      res.end();
      return;
    }
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
    const r = asFetchResult(await executeFetch({ url: `${base}/ok` }, cfg()));
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
    const r = asFetchResult(await executeFetch({ url: `${base}/big`, max_bytes: 100 }, cfg()));
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.bytes_truncated).toBe(true);
      expect(r.body.length).toBe(100);
    }
  });

  it('re-runs the SSRF check on each redirect hop (302 → metadata is blocked)', async () => {
    const r = asFetchResult(await executeFetch({ url: `${base}/redirect-to-blocked` }, cfg()));
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.error).toBe('blocked_host');
      expect(r.message).toMatch(/169\.254\.169\.254/);
    }
  });

  it('follows a same-host relative redirect and records the chain', async () => {
    const r = asFetchResult(await executeFetch({ url: `${base}/redirect-relative` }, cfg()));
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.body).toBe('hello');
      expect(r.redirect_chain).toEqual([`${base}/redirect-relative`]);
    }
  });

  it('returns error:timeout when the server is slower than timeout_ms', async () => {
    const r = asFetchResult(await executeFetch({ url: `${base}/slow`, timeout_ms: 50 }, cfg()));
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('timeout');
  });

  it('sends a JSON body with content-type and parses the response', async () => {
    const r = asFetchResult(
      await executeFetch(
        { url: `${base}/echo`, method: 'POST', json: { a: 1 }, response_format: 'json' },
        cfg(),
      ),
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
    const r = asFetchResult(await executeFetch({ url: 'http://127.0.0.1:1/' }, cfg()));
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toBe('transport_error');
  });
});

describe('executeFetch page-reading mode (format set)', () => {
  it('converts an HTML page to markdown', async () => {
    const r = asFetchResult(await executeFetch({ url: `${base}/html`, format: 'markdown' }, cfg()));
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.body).toBe('# Title\n\npara');
      expect(r.transformed).toBe('markdown');
      expect(r.content_type).toBe('text/html');
      expect(r.response_format).toBe('text');
    }
  });

  it('extracts plain text from an HTML page with format:"text"', async () => {
    const r = asFetchResult(await executeFetch({ url: `${base}/html`, format: 'text' }, cfg()));
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.body).toBe('Title\npara');
      expect(r.transformed).toBe('text');
    }
  });

  it('transforms application/xhtml+xml pages too (advertised in the Accept header)', async () => {
    const r = asFetchResult(
      await executeFetch({ url: `${base}/xhtml`, format: 'markdown' }, cfg()),
    );
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.body).toBe('# XTitle');
      expect(r.transformed).toBe('markdown');
      expect(r.content_type).toBe('application/xhtml+xml');
    }
  });

  it('returns raw HTML with format:"html" (no transform)', async () => {
    const r = asFetchResult(await executeFetch({ url: `${base}/html`, format: 'html' }, cfg()));
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.body).toContain('<h1>Title</h1>');
      expect(r.transformed).toBeUndefined();
      expect(r.content_type).toBe('text/html');
    }
  });

  it('passes non-HTML bodies through unchanged even with format:"markdown"', async () => {
    const r = asFetchResult(await executeFetch({ url: `${base}/ok`, format: 'markdown' }, cfg()));
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.body).toBe('hello');
      expect(r.transformed).toBeUndefined();
      expect(r.content_type).toBe('text/plain');
    }
  });

  it('sends browser UA + format Accept + accept-language when format is set', async () => {
    await executeFetch({ url: `${base}/html`, format: 'markdown' }, cfg());
    expect(lastReceivedHeaders['user-agent']).toBe(BROWSER_USER_AGENT);
    expect(lastReceivedHeaders.accept).toBe(
      'text/markdown;q=1.0, text/x-markdown;q=0.9, text/plain;q=0.8, text/html;q=0.7, */*;q=0.1',
    );
    expect(lastReceivedHeaders['accept-language']).toBe('en-US,en;q=0.9');
  });

  it('keeps the configured honest UA when format is absent', async () => {
    await executeFetch({ url: `${base}/ok` }, cfg());
    expect(lastReceivedHeaders['user-agent']).toMatch(/iii-harness/);
    expect(lastReceivedHeaders.accept).toBeUndefined();
  });

  it('caller-supplied user-agent and accept win over page-mode injection', async () => {
    await executeFetch(
      {
        url: `${base}/html`,
        format: 'markdown',
        headers: { 'user-agent': 'my-bot/1.0', accept: 'text/x-custom' },
      },
      cfg(),
    );
    expect(lastReceivedHeaders['user-agent']).toBe('my-bot/1.0');
    expect(lastReceivedHeaders.accept).toBe('text/x-custom');
  });

  it('retries once with the honest UA on a Cloudflare challenge (403 + cf-mitigated)', async () => {
    cfChallengeHits = 0;
    const r = asFetchResult(
      await executeFetch({ url: `${base}/cf-challenge`, format: 'markdown' }, cfg()),
    );
    expect(cfChallengeHits).toBe(1); // exactly one challenged attempt
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.status).toBe(200);
      expect(r.body).toBe('welcome human');
    }
  });

  it('does NOT retry on the Cloudflare challenge when format is absent (curl path)', async () => {
    cfChallengeHits = 0;
    const r = asFetchResult(
      await executeFetch(
        { url: `${base}/cf-challenge`, headers: { 'user-agent': BROWSER_USER_AGENT } },
        cfg(),
      ),
    );
    expect(cfChallengeHits).toBe(1);
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.status).toBe(403); // challenge returned as-is, no retry
  });

  it('returns the 403 envelope when the honest-UA retry is ALSO challenged', async () => {
    cfChallengeHits = 0;
    const r = asFetchResult(
      await executeFetch({ url: `${base}/cf-always`, format: 'markdown' }, cfg()),
    );
    expect(cfChallengeHits).toBe(2); // one attempt + exactly one retry, no loop
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.status).toBe(403);
  });

  it('does NOT retry on a 403 whose cf-mitigated is not exactly "challenge"', async () => {
    cfChallengeHits = 0;
    const r = asFetchResult(
      await executeFetch({ url: `${base}/cf-managed`, format: 'markdown' }, cfg()),
    );
    expect(cfChallengeHits).toBe(1);
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.status).toBe(403);
  });

  it('does NOT retry the Cloudflare challenge for non-idempotent methods (no POST replay)', async () => {
    cfChallengeHits = 0;
    const r = asFetchResult(
      await executeFetch(
        { url: `${base}/cf-always`, method: 'POST', body: 'payload', format: 'markdown' },
        cfg(),
      ),
    );
    expect(cfChallengeHits).toBe(1); // single attempt — a replayed POST would duplicate the action
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.status).toBe(403);
  });

  it('records the redirect chain on an image envelope', async () => {
    const r = await executeFetch({ url: `${base}/redirect-to-img`, format: 'markdown' }, cfg());
    expect(isImageEnvelope(r)).toBe(true);
    if (isImageEnvelope(r)) {
      expect(r.details.redirect_chain).toEqual([`${base}/redirect-to-img`]);
    }
  });

  it('does NOT retry when the caller supplied their own user-agent', async () => {
    cfChallengeHits = 0;
    const r = asFetchResult(
      await executeFetch(
        {
          url: `${base}/cf-challenge`,
          format: 'markdown',
          headers: { 'user-agent': BROWSER_USER_AGENT },
        },
        cfg(),
      ),
    );
    expect(cfChallengeHits).toBe(1);
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.status).toBe(403);
  });

  it('returns an image envelope (image block + text fallback) for image responses', async () => {
    const r = await executeFetch({ url: `${base}/img.png`, format: 'markdown' }, cfg());
    expect(isImageEnvelope(r)).toBe(true);
    if (isImageEnvelope(r)) {
      const image = r.content.find((c) => c.type === 'image');
      const textBlock = r.content.find((c) => c.type === 'text');
      expect(image).toBeDefined();
      if (image?.type === 'image') {
        expect(image.mime).toBe('image/png');
        expect(Buffer.from(image.data, 'base64').equals(PNG_BYTES)).toBe(true);
      }
      if (textBlock?.type === 'text') {
        expect(textBlock.text).toMatch(/Image fetched \(image\/png, \d+ bytes\)/);
      }
      expect(r.details.ok).toBe(true);
      expect(r.details.status).toBe(200);
      expect(r.details.content_type).toBe('image/png');
      expect(r.details.bytes).toBe(PNG_BYTES.length);
    }
  });

  it('falls back to the raw body when the markdown transform blows up (hostile nesting)', async () => {
    const r = asFetchResult(
      await executeFetch({ url: `${base}/deep-html`, format: 'markdown' }, cfg()),
    );
    expect(r.ok).toBe(true); // never throws — the envelope is the contract
    if (r.ok) {
      expect(r.body).toContain('core'); // raw HTML returned untransformed
      expect(r.body).toContain('<div>');
      expect(r.transformed).toBeUndefined();
      expect(r.content_type).toBe('text/html');
    }
  });

  it('format wins over response_format (markdown beats base64)', async () => {
    const r = asFetchResult(
      await executeFetch(
        { url: `${base}/html`, format: 'markdown', response_format: 'base64' },
        cfg(),
      ),
    );
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.body).toBe('# Title\n\npara'); // markdown text, not base64
      expect(r.response_format).toBe('text');
    }
  });

  it('images are NOT special-cased without format (base64 transport as before)', async () => {
    const r = asFetchResult(
      await executeFetch({ url: `${base}/img.png`, response_format: 'base64' }, cfg()),
    );
    expect(r.ok).toBe(true);
    if (r.ok) expect(r.body).toBe(PNG_BYTES.toString('base64'));
  });
});

describe('image envelope hardening (unviewable images fall back to the base64 envelope)', () => {
  it('does NOT launder an error-status image into an image block', async () => {
    const r = asFetchResult(
      await executeFetch({ url: `${base}/img-404`, format: 'markdown' }, cfg()),
    );
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.status).toBe(404); // caller can see the failure
      expect(r.body).toBe(PNG_BYTES.toString('base64'));
      expect(r.response_format).toBe('base64');
      expect(r.content_type).toBe('image/png');
    }
  });

  it('does NOT emit non-allowlisted image types (svg) as image blocks', async () => {
    const r = asFetchResult(
      await executeFetch({ url: `${base}/img-svg`, format: 'markdown' }, cfg()),
    );
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.content_type).toBe('image/svg+xml');
      expect(r.response_format).toBe('base64');
    }
  });

  it('does NOT emit a truncated image as an image block (corrupt bytes)', async () => {
    const r = asFetchResult(
      await executeFetch({ url: `${base}/img.png`, format: 'markdown', max_bytes: 10 }, cfg()),
    );
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.bytes_truncated).toBe(true);
      expect(r.response_format).toBe('base64');
    }
  });

  it('does NOT emit an empty image body as an image block', async () => {
    const r = asFetchResult(
      await executeFetch({ url: `${base}/img-empty`, format: 'markdown' }, cfg()),
    );
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.body).toBe('');
      expect(r.response_format).toBe('base64');
    }
  });
});

describe('response byte caps', () => {
  it('does NOT apply default_response_bytes to a raw fetch (regression: no silent truncation)', async () => {
    // A raw fetch (no `format`) that omits max_bytes must default to the
    // hard ceiling, not the small page-mode cap — otherwise existing
    // API/download callers silently truncate and consume partial bodies.
    const tiny = loadWebConfig({ web: { default_response_bytes: 100 } });
    const r = asFetchResult(await executeFetch({ url: `${base}/big` }, tiny));
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.bytes_truncated).toBe(false);
      expect(r.body.length).toBe(10_000);
    }
  });

  it('applies default_response_bytes in page-reading mode when the caller passes no max_bytes', async () => {
    const tiny = loadWebConfig({ web: { default_response_bytes: 100 } });
    const r = asFetchResult(await executeFetch({ url: `${base}/big`, format: 'markdown' }, tiny));
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.bytes_truncated).toBe(true);
      expect(r.body.length).toBe(100);
    }
  });

  it('lets an explicit max_bytes exceed the default (up to the ceiling)', async () => {
    const tiny = loadWebConfig({ web: { default_response_bytes: 100 } });
    const r = asFetchResult(await executeFetch({ url: `${base}/big`, max_bytes: 10_000 }, tiny));
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.bytes_truncated).toBe(false);
      expect(r.body.length).toBe(10_000);
    }
  });

  it('still clamps an explicit max_bytes to the hard ceiling', async () => {
    const capped = loadWebConfig({ web: { max_response_bytes: 100 } });
    const r = asFetchResult(await executeFetch({ url: `${base}/big`, max_bytes: 10_000 }, capped));
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.bytes_truncated).toBe(true);
      expect(r.body.length).toBe(100);
    }
  });
});

describe('transform bounds', () => {
  it('skips the HTML transform above max_transform_bytes (event-loop protection)', async () => {
    const small = loadWebConfig({ web: { max_transform_bytes: 10 } });
    const r = asFetchResult(await executeFetch({ url: `${base}/html`, format: 'markdown' }, small));
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.body).toContain('<h1>Title</h1>'); // raw, untransformed
      expect(r.transformed).toBeUndefined();
    }
  });

  it('appends a visible truncation notice to a transformed truncated body', async () => {
    const r = asFetchResult(
      await executeFetch({ url: `${base}/html`, format: 'markdown', max_bytes: 70 }, cfg()),
    );
    expect(r.ok).toBe(true);
    if (r.ok) {
      expect(r.bytes_truncated).toBe(true);
      expect(r.transformed).toBe('markdown');
      expect(r.body).toContain('[Content truncated at max_bytes');
    }
  });
});
