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

export const PageFormatSchema = z.enum(['markdown', 'text', 'html']);
export type PageFormat = z.infer<typeof PageFormatSchema>;

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
      'Cap on response body bytes. Defaults to the worker ceiling (5 MiB) for raw fetches, or a context-safe 256 KiB in page-reading mode (`format` set); pass an explicit value to override (up to the 5 MiB ceiling). Larger responses are truncated and bytes_truncated:true is returned.',
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
  format: PageFormatSchema.optional().describe(
    'Page-reading mode for fetching web pages (not APIs). When set, the request goes out with a browser User-Agent and a format-matched Accept header, and HTML responses are transformed: "markdown" converts HTML to Markdown (best for reading pages), "text" extracts plain text, "html" returns raw HTML. Non-HTML bodies pass through unchanged; image responses come back as an image the model can view. Omit for raw API/curl-style fetches — when set, response_format is ignored and treated as "text".',
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
      /** Response content-type mime (lower-cased, no parameters). Set when `format` was requested. */
      content_type?: string;
      /** Echoes the page format whose HTML transform actually ran (html→markdown or html→text). */
      transformed?: PageFormat;
    }
  | {
      ok: false;
      error: FetchErrorCode;
      message: string;
      status?: number;
    };

/**
 * Returned instead of `FetchResult` when `format` is set and the response
 * is an image. Shaped like the orchestrator's FunctionResult envelope
 * ({content, details}) so `decodeOrPassthrough` preserves the image block
 * and providers that support tool-result images (Anthropic) render it to
 * the model; text-only providers fall back to the text line.
 */
export type FetchImageResult = {
  content: Array<{ type: 'image'; mime: string; data: string } | { type: 'text'; text: string }>;
  details: {
    ok: true;
    status: number;
    status_text: string;
    content_type: string;
    bytes: number;
    redirect_chain?: string[];
  };
};

const TOOL_DESCRIPTION = [
  'Fetch a URL over HTTP(S) and return the response as a structured envelope.',
  'Use this INSTEAD of `shell::exec` with curl for any HTTP request — it',
  'returns {ok, status, headers, body} as JSON, enforces size/timeout caps,',
  'and blocks private / cloud-metadata / link-local addresses server-side',
  '(SSRF guard; loopback is allowed by default for harness dev workflows).',
  'To READ A WEB PAGE, set `format: "markdown"` — HTML is converted to',
  'Markdown and images come back viewable (image responses in that mode',
  'return the image itself plus a text line, not the {ok,...} envelope).',
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
