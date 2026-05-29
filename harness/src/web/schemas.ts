/**
 * Wire schema + tool description for `web::fetch`. Zod validates
 * ingress; `zodToJsonSchema` exports the same shape so the model sees
 * a proper tool-call schema in `request_format`.
 *
 * The description steers the model AWAY from `shell::exec` curl —
 * without that line the LLM keeps reaching for curl from training-
 * data habit. Same lever `shell::exec`'s own description uses to keep
 * the model from passing argv as an array.
 */

import type { RegisterFunctionOptions } from 'iii-sdk';
import { z } from 'zod';
import { zodToJsonSchema } from 'zod-to-json-schema';

// Case-insensitive: 'get' / 'Get' / 'GET' all accepted, normalised to upper.
export const HttpMethodSchema = z.preprocess(
  (v) => (typeof v === 'string' ? v.toUpperCase() : v),
  z.enum(['GET', 'HEAD', 'POST', 'PUT', 'PATCH', 'DELETE', 'OPTIONS']),
);
export type HttpMethod = z.infer<typeof HttpMethodSchema>;

export const ResponseFormatSchema = z.enum(['text', 'base64', 'json']);
export type ResponseFormat = z.infer<typeof ResponseFormatSchema>;

export const FetchPayloadSchema = z.object({
  url: z.string().min(1).describe('Absolute http(s):// URL to fetch.'),
  method: HttpMethodSchema.optional().describe(
    'HTTP method. Defaults to GET. Case-insensitive ("get" works).',
  ),
  headers: z
    .record(z.string(), z.string())
    .optional()
    .describe('Request headers. Lower-cased keys recommended.'),
  body: z
    .string()
    .optional()
    .describe('Raw request body as a string. Prefer `json` for JSON payloads.'),
  json: z
    .unknown()
    .optional()
    .describe(
      'Structured JSON payload. When set, the handler stringifies it and forces `content-type: application/json` — do NOT also set `body`. If both are present, `json` wins.',
    ),
  timeout_ms: z
    .number()
    .int()
    .positive()
    .optional()
    .describe("Per-request timeout in ms. Capped by the worker's max_timeout_ms."),
  max_bytes: z
    .number()
    .int()
    .positive()
    .optional()
    .describe(
      'Cap on response body bytes. Larger responses are truncated and bytes_truncated:true is returned.',
    ),
  follow_redirects: z
    .boolean()
    .optional()
    .describe(
      'Follow 3xx redirects. Defaults to true. Each hop is re-checked against the SSRF blocklist.',
    ),
  response_format: ResponseFormatSchema.optional().describe(
    'How to return the response body: "text" (default), "base64" for binary, or "json" to auto-parse application/json responses into the `json` field.',
  ),
});
export type FetchPayload = z.infer<typeof FetchPayloadSchema>;

export const FetchErrorCodeSchema = z.enum([
  'invalid_payload',
  'invalid_url',
  'blocked_host',
  'timeout',
  // Reserved: oversize responses are TRUNCATED (bytes_truncated:true), not
  // errored — the worker does not currently emit `too_large`. Kept in the
  // wire contract for forward-compat and parser tolerance.
  'too_large',
  'too_many_redirects',
  'transport_error',
]);
export type FetchErrorCode = z.infer<typeof FetchErrorCodeSchema>;

export type FetchResult =
  | {
      ok: true;
      status: number;
      status_text: string;
      headers: Record<string, string>;
      body: string;
      /** Set when response_format is "json" and the body parsed cleanly. */
      json?: unknown;
      /** Set when response_format is "json" but parsing failed. */
      parse_error?: string;
      response_format: ResponseFormat;
      bytes_truncated: boolean;
      /** The chain of intermediate URLs walked before the final response. Omitted when no redirects happened. */
      redirect_chain?: string[];
    }
  | {
      ok: false;
      error: FetchErrorCode;
      message: string;
      status?: number;
    };

const TOOL_DESCRIPTION = [
  'Fetch a URL over HTTP(S) and return the response as a structured envelope.',
  'Use this INSTEAD of `shell::exec` with curl for any HTTP request — it',
  'returns {ok, status, headers, body} as JSON, enforces size/timeout caps,',
  'and blocks private / cloud-metadata / link-local addresses server-side',
  '(SSRF guard; loopback is allowed by default for harness dev workflows).',
  'For JSON: pass `json: {...}` (auto-stringifies + sets content-type) and',
  '`response_format: "json"` (auto-parses response into the `json` field).',
  'Method is case-insensitive. On failure returns `{ok:false, error, message}`',
  'where `error` is one of: invalid_payload, invalid_url, blocked_host, timeout,',
  'too_many_redirects, transport_error. Branch on `error`, not on text.',
].join(' ');

export const fetchFunctionOptions: RegisterFunctionOptions = {
  description: TOOL_DESCRIPTION,
  request_format: zodToJsonSchema(FetchPayloadSchema, { name: 'FetchPayload' }) as Record<
    string,
    unknown
  >,
};
