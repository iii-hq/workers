/**
 * The actual HTTP call for `web::fetch`.
 *
 * SSRF-safe IP pinning: the host is resolved + validated ONCE in
 * `ssrf.ts::checkTarget`, then the socket is pinned to that exact IP via
 * the `lookup` request option — node never re-resolves, defeating DNS
 * rebinding (TOCTOU between the check and the connect). Crucially the
 * request URL stays on the ORIGINAL hostname, so SNI, TLS certificate
 * validation, and the `Host` header all bind to the real host. (An
 * earlier version rewrote the URL to the IP literal, which made HTTPS
 * fail certificate validation against the IP — see fetch.integration.test.)
 *
 * Built on `node:http` / `node:https` (not global `fetch`) because only the
 * node request API exposes the per-request `lookup` + `servername` hooks the
 * pinning needs; undici is not a dependency here. Adds:
 *   - manual redirect handling so each hop re-runs the SSRF check,
 *   - streamed body read with a hard byte cap (truncate-and-mark, not throw),
 *   - cross-origin / scheme-downgrade Authorization + Cookie strip on redirect.
 */

import { Buffer } from 'node:buffer';
import * as nodeHttp from 'node:http';
import type { IncomingHttpHeaders, IncomingMessage } from 'node:http';
import * as nodeHttps from 'node:https';
import type { Readable } from 'node:stream';
import { logger } from '../runtime/otel.js';
import type { WebConfig } from './config.js';
import type { FetchPayload, FetchResult, ResponseFormat } from './schemas.js';
import { type ParsedTarget, type SsrfPolicy, checkTarget, parseTarget } from './ssrf.js';

const HEADER_DENY_ON_REDIRECT = new Set(['authorization', 'cookie', 'proxy-authorization']);

type RawResponse = {
  status: number;
  statusText: string;
  headers: Record<string, string>;
  bytes: Buffer;
  truncated: boolean;
};

type RequestOutcome =
  | { kind: 'response'; resp: RawResponse }
  | { kind: 'timeout' }
  | { kind: 'transport_error'; message: string };

/**
 * Read a node Readable (IncomingMessage) into a Buffer, stopping at `cap`
 * bytes and marking `truncated`. Mirrors the web-stream capped reader it
 * replaced; exported for unit tests.
 */
export function readIncomingCapped(
  stream: Readable,
  cap: number,
): Promise<{ bytes: Buffer; truncated: boolean }> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let total = 0;
    let settled = false;

    const finish = (truncated: boolean) => {
      if (settled) return;
      settled = true;
      resolve({ bytes: Buffer.concat(chunks), truncated });
    };

    stream.on('data', (raw: Buffer | string) => {
      if (settled) return;
      const chunk = Buffer.isBuffer(raw) ? raw : Buffer.from(raw);
      if (total + chunk.length > cap) {
        const remaining = cap - total;
        if (remaining > 0) chunks.push(chunk.subarray(0, remaining));
        // Resolve BEFORE destroy so a post-destroy 'error'/'close' can't
        // flip the outcome — the cap is the contract.
        finish(true);
        stream.destroy();
        return;
      }
      chunks.push(chunk);
      total += chunk.length;
    });
    stream.on('end', () => finish(false));
    stream.on('close', () => finish(false));
    stream.on('error', (err) => {
      if (settled) return;
      settled = true;
      reject(err);
    });
  });
}

function flattenIncomingHeaders(h: IncomingHttpHeaders): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(h)) {
    if (v === undefined) continue;
    // node already lower-cases keys; set-cookie arrives as an array.
    out[k.toLowerCase()] = Array.isArray(v) ? v.join(', ') : v;
  }
  return out;
}

/**
 * `lookup` shim that forces every connection attempt onto the pre-validated
 * IP, ignoring the hostname node would otherwise resolve. Matches the
 * `net.LookupFunction` contract, including the `{ all: true }` variant.
 */
function pinnedLookup(address: string, family: 4 | 6): nodeHttps.RequestOptions['lookup'] {
  return ((hostname: string, options: unknown, callback: unknown) => {
    void hostname;
    const opts = (options ?? {}) as { all?: boolean };
    if (opts.all) {
      (callback as (e: Error | null, addrs: Array<{ address: string; family: number }>) => void)(
        null,
        [{ address, family }],
      );
    } else {
      (callback as (e: Error | null, addr: string, fam: number) => void)(null, address, family);
    }
  }) as nodeHttps.RequestOptions['lookup'];
}

/** Drop any caller-supplied Host header so node derives it from the URL. */
function withoutHostHeader(headers: Record<string, string>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(headers)) {
    if (k.toLowerCase() === 'host') continue;
    out[k] = v;
  }
  return out;
}

function performRequest(
  parsed: ParsedTarget,
  address: string,
  family: 4 | 6,
  method: string,
  headers: Record<string, string>,
  body: string | undefined,
  timeoutMs: number,
  maxBytes: number,
): Promise<RequestOutcome> {
  const isHttps = parsed.url.protocol === 'https:';
  const mod = isHttps ? nodeHttps : nodeHttp;

  return new Promise<RequestOutcome>((resolve) => {
    let settled = false;
    const done = (o: RequestOutcome) => {
      if (settled) return;
      settled = true;
      resolve(o);
    };

    const options: nodeHttps.RequestOptions = {
      method,
      headers: withoutHostHeader(headers),
      lookup: pinnedLookup(address, family),
    };
    // SNI + certificate identity bind to the real hostname (not the IP).
    if (isHttps) options.servername = parsed.hostname;

    const req = mod.request(parsed.url, options, (res: IncomingMessage) => {
      readIncomingCapped(res, maxBytes).then(
        ({ bytes, truncated }) =>
          done({
            kind: 'response',
            resp: {
              status: res.statusCode ?? 0,
              statusText: res.statusMessage ?? '',
              headers: flattenIncomingHeaders(res.headers),
              bytes,
              truncated,
            },
          }),
        (err) =>
          done({
            kind: 'transport_error',
            message: err instanceof Error ? err.message : String(err),
          }),
      );
    });

    req.setTimeout(timeoutMs, () => {
      req.destroy(
        Object.assign(new Error(`request timed out after ${timeoutMs}ms`), { __timeout: true }),
      );
    });
    req.on('error', (err: Error & { __timeout?: boolean }) => {
      if (err.__timeout) done({ kind: 'timeout' });
      else done({ kind: 'transport_error', message: err.message });
    });

    if (body !== undefined && method !== 'GET' && method !== 'HEAD') req.write(body);
    req.end();
  });
}

function encodeBody(bytes: Buffer, format: ResponseFormat): string {
  if (format === 'base64') return bytes.toString('base64');
  // 'json' falls through here too — we always return the raw text in
  // `body` and additionally surface the parsed value in `json` (see
  // tryParseJson below). This keeps `body` typed as `string` and gives
  // the agent both the structured value and the original text.
  return bytes.toString('utf8');
}

function tryParseJson(
  text: string,
): { value: unknown; error?: undefined } | { value?: undefined; error: string } {
  try {
    return { value: JSON.parse(text) };
  } catch (err) {
    return { error: err instanceof Error ? err.message : String(err) };
  }
}

/**
 * If `payload.json` is set, force the request to a JSON shape:
 *   - body = JSON.stringify(payload.json)
 *   - content-type = application/json (only if the caller didn't set one)
 * Returns the effective body string and headers. `payload.json` wins
 * over `payload.body` when both are set — the description warns the
 * caller about this.
 */
function applyJsonPayload(
  payload: FetchPayload,
  headers: Record<string, string>,
): { body: string | undefined; headers: Record<string, string> } {
  if (payload.json === undefined) {
    return { body: payload.body, headers };
  }
  const lowered: Record<string, string> = {};
  for (const [k, v] of Object.entries(headers)) lowered[k.toLowerCase()] = v;
  if (!lowered['content-type']) {
    headers = { ...headers, 'content-type': 'application/json' };
  }
  return { body: JSON.stringify(payload.json), headers };
}

export function stripCrossOriginAuth(
  headers: Record<string, string>,
  from: URL,
  to: URL,
): Record<string, string> {
  // Strip auth/cookie when the redirect leaves the origin OR downgrades the
  // scheme (https → http). A same-host downgrade still leaks credentials in
  // cleartext, so host equality alone is not enough.
  const sameHost = from.host === to.host;
  const schemeDowngrade = from.protocol === 'https:' && to.protocol === 'http:';
  if (sameHost && !schemeDowngrade) return headers;
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(headers)) {
    if (HEADER_DENY_ON_REDIRECT.has(k.toLowerCase())) continue;
    out[k] = v;
  }
  return out;
}

export async function executeFetch(payload: FetchPayload, cfg: WebConfig): Promise<FetchResult> {
  const t0 = Date.now();
  const method = payload.method ?? 'GET';
  const followRedirects = payload.follow_redirects ?? true;
  const responseFormat: ResponseFormat = payload.response_format ?? 'text';
  const timeoutMs = Math.min(payload.timeout_ms ?? cfg.max_timeout_ms, cfg.max_timeout_ms);
  const maxBytes = Math.min(payload.max_bytes ?? cfg.max_response_bytes, cfg.max_response_bytes);

  let currentUrl = payload.url;
  const baseHeaders: Record<string, string> = {
    'user-agent': cfg.user_agent,
    ...(payload.headers ?? {}),
  };
  const jsonApplied = applyJsonPayload(payload, baseHeaders);
  let currentHeaders = jsonApplied.headers;
  const effectiveBody = jsonApplied.body;
  const redirectChain: string[] = [];
  const policy: SsrfPolicy = { allow_loopback: cfg.allow_loopback };

  const telemetryHost = (() => {
    try {
      return new URL(payload.url).host;
    } catch {
      return 'unparseable';
    }
  })();

  const logResult = (result: FetchResult): FetchResult => {
    const ms = Date.now() - t0;
    if (result.ok) {
      logger.info('web::fetch ok', {
        host: telemetryHost,
        method,
        status: result.status,
        bytes_truncated: result.bytes_truncated,
        redirects: result.redirect_chain?.length ?? 0,
        response_format: result.response_format,
        ms,
      });
    } else {
      logger.warn('web::fetch error', { host: telemetryHost, method, error: result.error, ms });
    }
    return result;
  };

  for (let hop = 0; hop <= cfg.max_redirects; hop++) {
    const parsed = parseTarget(currentUrl);
    if ('error' in parsed) {
      return logResult({ ok: false, error: 'invalid_url', message: parsed.error });
    }
    const check = await checkTarget(parsed, policy);
    if (!check.ok) {
      return logResult(check);
    }

    const outcome = await performRequest(
      parsed,
      check.address,
      check.family,
      method,
      currentHeaders,
      effectiveBody,
      timeoutMs,
      maxBytes,
    );
    if (outcome.kind === 'timeout') {
      return logResult({
        ok: false,
        error: 'timeout',
        message: `request timed out after ${timeoutMs}ms (raise timeout_ms or shrink response with smaller max_bytes)`,
      });
    }
    if (outcome.kind === 'transport_error') {
      return logResult({ ok: false, error: 'transport_error', message: outcome.message });
    }
    const resp = outcome.resp;

    if (followRedirects && resp.status >= 300 && resp.status < 400) {
      const location = resp.headers.location;
      if (location) {
        let nextUrl: URL;
        try {
          nextUrl = new URL(location, parsed.url);
        } catch {
          return logResult({
            ok: false,
            error: 'transport_error',
            message: `redirect target is not a valid URL: ${location}`,
          });
        }
        if (hop === cfg.max_redirects) {
          return logResult({
            ok: false,
            error: 'too_many_redirects',
            message: `exceeded max_redirects (${cfg.max_redirects})`,
          });
        }
        redirectChain.push(currentUrl);
        currentHeaders = stripCrossOriginAuth(currentHeaders, parsed.url, nextUrl);
        currentUrl = nextUrl.toString();
        continue;
      }
      // 3xx without Location — fall through and return the response.
    }

    const body = encodeBody(resp.bytes, responseFormat);
    const result: FetchResult = {
      ok: true,
      status: resp.status,
      status_text: resp.statusText,
      headers: resp.headers,
      body,
      response_format: responseFormat,
      bytes_truncated: resp.truncated,
    };
    if (redirectChain.length > 0) result.redirect_chain = redirectChain;
    if (responseFormat === 'json' && !resp.truncated) {
      const json = tryParseJson(body);
      if ('error' in json) result.parse_error = json.error;
      else result.json = json.value;
    }
    return logResult(result);
  }

  return logResult({
    ok: false,
    error: 'too_many_redirects',
    message: `exceeded max_redirects (${cfg.max_redirects})`,
  });
}
