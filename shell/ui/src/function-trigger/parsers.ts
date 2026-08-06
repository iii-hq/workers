import { z } from 'zod'
import {
  collectErrorCandidates,
  extractFirstJsonObject,
  fsEntrySchema,
  fsMatchSchema,
  fsSedFileResultSchema,
  safeParseRequest,
  safeParseResponse,
  unwrapEnvelope,
} from '../lib/envelope'
import { type ErrorDisplay, parseErrorDisplay } from '../lib/error-display'

/* Zod schemas mirroring the shell::* request/response shapes from
   `workers/shell/src/{functions/types,jobs,target,configuration}.rs` and
   `src/fs/{mod,wire,error}.rs`.

   The Rust handlers deserialize plain JSON via serde, so wire payloads
   always match these shapes. Every schema is intentionally non-strict
   (no `.strict()`); forward-compat is preserved for unknown future
   fields (notably the engine-injected `_caller_worker_id`) and the
   renderers only read what they need. */

// Re-exports so the shell views import everything from one place.
export type { FsEntry, FsMatch, FsSedFileResult } from '../lib/envelope'
export type { ErrorDisplay } from '../lib/error-display'
export {
  extractFirstJsonObject,
  fsEntrySchema,
  fsMatchSchema,
  fsSedFileResultSchema,
  safeParseRequest,
  safeParseResponse,
  unwrapEnvelope,
}

// ---------------------------------------------------------------------------
// Function ids
// ---------------------------------------------------------------------------

export const SHELL_FUNCTION_IDS = [
  'shell::exec',
  'shell::exec_bg',
  'shell::status',
  'shell::kill',
  'shell::list',
  'shell::config-status',
  'shell::fs::ls',
  'shell::fs::stat',
  'shell::fs::mkdir',
  'shell::fs::write',
  'shell::fs::read',
  'shell::fs::rm',
  'shell::fs::chmod',
  'shell::fs::mv',
  'shell::fs::grep',
  'shell::fs::sed',
] as const

export type ShellFunctionId = (typeof SHELL_FUNCTION_IDS)[number]

const SHELL_FUNCTION_ID_SET: ReadonlySet<string> = new Set<string>(
  SHELL_FUNCTION_IDS,
)

export function isShellFunction(id: string): id is ShellFunctionId {
  return SHELL_FUNCTION_ID_SET.has(id)
}

// ---------------------------------------------------------------------------
// Shared building blocks
// ---------------------------------------------------------------------------

/** `src/target.rs::Target` — internally tagged on `kind`, snake_case.
    Requests carry `#[serde(default)]` on a non-Option field: an omitted
    key means host, but an explicit `null` is a server-side deserialize
    error — so the field is `.optional()`, NOT `.nullable()`. `sandbox_id`
    is a UUID on the server; kept a lenient string here so a malformed id
    still renders. Unknown `kind` values fail parse (raw-JSON fallback),
    matching the server's deserialize error. */
export const targetSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('host') }),
  z.object({ kind: z.literal('sandbox'), sandbox_id: z.string() }),
])
export type ShellTarget = z.infer<typeof targetSchema>

/** `src/jobs.rs::JobStatus` — `rename_all = "lowercase"`. */
export const jobStatusSchema = z.enum([
  'running',
  'finished',
  'killed',
  'failed',
])
export type JobStatus = z.infer<typeof jobStatusSchema>

/** `src/jobs.rs::JobRecord` — no renames, no skip_serializing_if: all 10
    keys always present; `finished_at_ms`/`exit_code` are explicit JSON
    null when unset ⇒ `.nullable()`, NOT `.optional()`. */
export const jobRecordSchema = z.object({
  id: z.string(),
  argv: z.array(z.string()),
  started_at_ms: z.number(),
  finished_at_ms: z.number().nullable(),
  status: jobStatusSchema,
  exit_code: z.number().nullable(),
  stdout: z.string(),
  stderr: z.string(),
  stdout_truncated: z.boolean(),
  stderr_truncated: z.boolean(),
})
export type JobRecord = z.infer<typeof jobRecordSchema>

/** `src/fs/mod.rs::ContentRef`. `direction` is optional on requests
    (serde default "read") and always present on fs::read responses —
    one schema covers both sides. */
export const contentRefSchema = z.object({
  channel_id: z.string(),
  access_key: z.string(),
  direction: z.enum(['read', 'write']).optional(),
})
export type ContentRef = z.infer<typeof contentRefSchema>

/** `WriteContentWire` — `#[serde(untagged)]`: inline UTF-8 string
    (host-only) or a streaming channel ref. */
export const writeContentSchema = z.union([z.string(), contentRefSchema])
export type WriteContent = z.infer<typeof writeContentSchema>

// ---------------------------------------------------------------------------
// shell::exec / shell::exec_bg
// ---------------------------------------------------------------------------

/** `ExecRequest`/`ExecBgRequest` (`src/functions/types.rs`).
    - `command` is string-only on the server (array → recovery-hint
      error), so `z.string()`; a request that errored server-side simply
      won't render as a terminal card (raw-JSON fallback is correct there).
    - `args`: absent/null ⇒ shell-words tokenization of `command`; present
      (even `[]`) ⇒ verbatim argv. The null/[] distinction is semantic —
      keep `.nullable().optional()` and DO NOT default to [].
    - `timeout_ms` is permissive server-side (any non-u64 silently ⇒
      default timeout, never an error) — mirror with `.catch(undefined)`
      so junk values don't kill the whole card.
    - `cwd`/`env`/`stdin` are `Option<T>` + `serde(default)` ⇒ both null
      and absent are legal on the wire: `.nullable().optional()`.
    - `env` is a BTreeMap: plain record. */
export const shellExecRequestSchema = z.object({
  command: z.string(),
  args: z.array(z.string()).nullable().optional(),
  timeout_ms: z.number().int().nonnegative().optional().catch(undefined),
  cwd: z.string().nullable().optional(),
  env: z.record(z.string(), z.string()).nullable().optional(),
  stdin: z.string().nullable().optional(),
  target: targetSchema.optional(),
})
export type ShellExecRequest = z.infer<typeof shellExecRequestSchema>

/** Same struct shape on the wire (`ExecBgRequest`). Alias, not copy. */
export const shellExecBgRequestSchema = shellExecRequestSchema
export type ShellExecBgRequest = ShellExecRequest

/** `ExecResponse` — all 7 keys always present; `exit_code` null when
    killed by signal/timeout. */
export const shellExecResponseSchema = z.object({
  exit_code: z.number().nullable(),
  stdout: z.string(),
  stderr: z.string(),
  duration_ms: z.number(),
  timed_out: z.boolean(),
  stdout_truncated: z.boolean(),
  stderr_truncated: z.boolean(),
})
export type ShellExecResponse = z.infer<typeof shellExecResponseSchema>

/** `ExecBgResponse` — `job_id` is `"job-<uuid>"`. */
export const shellExecBgResponseSchema = z.object({
  job_id: z.string(),
  argv: z.array(z.string()),
})
export type ShellExecBgResponse = z.infer<typeof shellExecBgResponseSchema>

// ---------------------------------------------------------------------------
// shell::status / shell::kill
// ---------------------------------------------------------------------------

export const shellStatusRequestSchema = z.object({ job_id: z.string() })
export type ShellStatusRequest = z.infer<typeof shellStatusRequestSchema>

export const shellStatusResponseSchema = z.object({ job: jobRecordSchema })
export type ShellStatusResponse = z.infer<typeof shellStatusResponseSchema>

export const shellKillRequestSchema = z.object({ job_id: z.string() })
export type ShellKillRequest = z.infer<typeof shellKillRequestSchema>

/** `KillResponse` — `reason` is the ONLY skip_serializing_if field in
    the family: key ABSENT when none ⇒ `.optional()`, NOT `.nullable()`. */
export const shellKillResponseSchema = z.object({
  job_id: z.string(),
  killed: z.boolean(),
  status: jobStatusSchema,
  reason: z.string().optional(),
})
export type ShellKillResponse = z.infer<typeof shellKillResponseSchema>

// ---------------------------------------------------------------------------
// shell::list / shell::config-status
// ---------------------------------------------------------------------------

/** Handler ignores the payload entirely; engine injects
    `_caller_worker_id`. */
export const shellListRequestSchema = z.object({}).passthrough()
export type ShellListRequest = z.infer<typeof shellListRequestSchema>

/** `JobSummary` — deliberately NO argv/stdout/stderr (cross-caller
    secrecy; full record only via shell::status). */
export const jobSummarySchema = z.object({
  id: z.string(),
  status: jobStatusSchema,
  started_at_ms: z.number(),
  finished_at_ms: z.number().nullable(),
  exit_code: z.number().nullable(),
  stdout_truncated: z.boolean(),
  stderr_truncated: z.boolean(),
})
export type JobSummary = z.infer<typeof jobSummarySchema>

export const shellListResponseSchema = z.object({
  jobs: z.array(jobSummarySchema),
  count: z.number(),
})
export type ShellListResponse = z.infer<typeof shellListResponseSchema>

export const shellConfigStatusRequestSchema = z.object({}).passthrough()
export type ShellConfigStatusRequest = z.infer<
  typeof shellConfigStatusRequestSchema
>

/** `configuration.rs::ReloadStatus` — `last_error` always present,
    explicit null. */
export const shellConfigStatusResponseSchema = z.object({
  last_outcome: z.enum(['applied', 'rejected']),
  last_error: z.string().nullable(),
  rejected_reloads: z.number(),
})
export type ShellConfigStatusResponse = z.infer<
  typeof shellConfigStatusResponseSchema
>

// ---------------------------------------------------------------------------
// Filesystem — shell::fs::* (`src/fs/mod.rs`). FsEntry/FsMatch/
// FsSedFileResult live in ../lib/envelope — they already encode the
// `file`→`path` alias and absent-`error`→null transforms.
// ---------------------------------------------------------------------------

/** ls / stat / read share one request shape. */
export const fsPathRequestSchema = z.object({
  target: targetSchema.optional(),
  path: z.string(),
})
export type FsPathRequest = z.infer<typeof fsPathRequestSchema>

export const fsLsRequestSchema = fsPathRequestSchema
export const fsStatRequestSchema = fsPathRequestSchema
export const fsReadRequestSchema = fsPathRequestSchema

export const fsLsResponseSchema = z.object({
  entries: z.array(fsEntrySchema),
})
export type FsLsResponse = z.infer<typeof fsLsResponseSchema>

/** `StatResponse` is `#[serde(transparent)]` — a bare FsEntry at the top
    level, no wrapper key. */
export const fsStatResponseSchema = fsEntrySchema
export type FsStatResponse = z.infer<typeof fsStatResponseSchema>

export const fsMkdirRequestSchema = z.object({
  target: targetSchema.optional(),
  path: z.string(),
  mode: z.string().optional(), // worker default "0755"
  parents: z.boolean().optional(), // default false
})
export type FsMkdirRequest = z.infer<typeof fsMkdirRequestSchema>

/** `path`/`already_existed` carry `#[serde(default)]` on the worker for
    sandbox-engine round-trips; be equally lenient here. Sandbox-target
    responses blank `path` and default the boolean — fill-ins, not
    signals (views suppress the pills via `isSandboxTarget`). */
export const fsMkdirResponseSchema = z.object({
  created: z.boolean(),
  path: z.string().optional().default(''),
  already_existed: z.boolean().optional().default(false),
})
export type FsMkdirResponse = z.infer<typeof fsMkdirResponseSchema>

export const fsRmRequestSchema = z.object({
  target: targetSchema.optional(),
  path: z.string(),
  recursive: z.boolean().optional(), // default false
})
export type FsRmRequest = z.infer<typeof fsRmRequestSchema>

export const fsRmResponseSchema = z.object({
  removed: z.boolean(),
  path: z.string().optional().default(''),
  was_present: z.boolean().optional().default(false), // host-only signal
})
export type FsRmResponse = z.infer<typeof fsRmResponseSchema>

export const fsChmodRequestSchema = z.object({
  target: targetSchema.optional(),
  path: z.string(),
  mode: z.string(), // REQUIRED, octal e.g. "0755"
  uid: z.number().nullable().optional(),
  gid: z.number().nullable().optional(),
  recursive: z.boolean().optional(), // default false
})
export type FsChmodRequest = z.infer<typeof fsChmodRequestSchema>

/** Callers receive `entries_changed`; `updated` is the legacy engine
    alias — accepted defensively (mirrors the worker's
    `#[serde(alias = "updated")]`), `entries_changed` wins. */
export const fsChmodResponseSchema = z
  .object({
    entries_changed: z.number().optional(),
    updated: z.number().optional(),
    path: z.string().optional().default(''),
    recursive: z.boolean().optional().default(false),
  })
  .refine((r) => r.entries_changed != null || r.updated != null)
  .transform((r) => ({
    entries_changed: r.entries_changed ?? r.updated ?? 0,
    path: r.path,
    recursive: r.recursive,
  }))
export type FsChmodResponse = z.infer<typeof fsChmodResponseSchema>

export const fsMvRequestSchema = z.object({
  target: targetSchema.optional(),
  src: z.string(),
  dst: z.string(),
  overwrite: z.boolean().optional(), // default false
})
export type FsMvRequest = z.infer<typeof fsMvRequestSchema>

export const fsMvResponseSchema = z.object({
  moved: z.boolean(),
  src: z.string().optional().default(''),
  dst: z.string().optional().default(''),
  overwrote: z.boolean().optional().default(false), // host best-effort
})
export type FsMvResponse = z.infer<typeof fsMvResponseSchema>

/** `recursive` defaults TRUE on the wire (unlike rm/chmod) — chips show
    the deviation ("non-recursive"). */
export const fsGrepRequestSchema = z.object({
  target: targetSchema.optional(),
  path: z.string(),
  pattern: z.string(),
  recursive: z.boolean().optional(), // default TRUE on the wire
  ignore_case: z.boolean().optional(),
  include_glob: z.array(z.string()).optional(),
  exclude_glob: z.array(z.string()).optional(),
  max_matches: z.number().optional(), // default 10000
  max_line_bytes: z.number().optional(), // default 4096
})
export type FsGrepRequest = z.infer<typeof fsGrepRequestSchema>

export const fsGrepResponseSchema = z.object({
  matches: z.array(fsMatchSchema),
  truncated: z.boolean(),
})
export type FsGrepResponse = z.infer<typeof fsGrepResponseSchema>

/** `recursive` and `regex` both default TRUE on the wire — chips show
    the deviations ("non-recursive", "literal"). */
export const fsSedRequestSchema = z.object({
  target: targetSchema.optional(),
  files: z.array(z.string()).optional(),
  path: z.string().nullable().optional(),
  recursive: z.boolean().optional(), // default TRUE
  include_glob: z.array(z.string()).optional(),
  exclude_glob: z.array(z.string()).optional(),
  pattern: z.string(),
  replacement: z.string(),
  regex: z.boolean().optional(), // default TRUE
  first_only: z.boolean().optional(),
  ignore_case: z.boolean().optional(),
})
export type FsSedRequest = z.infer<typeof fsSedRequestSchema>

export const fsSedResponseSchema = z.object({
  results: z.array(fsSedFileResultSchema),
  total_replacements: z.number(),
})
export type FsSedResponse = z.infer<typeof fsSedResponseSchema>

export const writeFileSpecSchema = z.object({
  path: z.string(),
  content: writeContentSchema,
  mode: z.string().optional(), // default "0644"
  parents: z.boolean().optional(),
})
export type WriteFileSpec = z.infer<typeof writeFileSpecSchema>

/** Single-file and batch forms share one lenient schema; the view
    branches on `files` being a non-empty array. (Mutual-exclusion is
    enforced server-side via S210, not by serde.) */
export const fsWriteRequestSchema = z.object({
  target: targetSchema.optional(),
  path: z.string().nullable().optional(),
  content: writeContentSchema.nullable().optional(),
  mode: z.string().nullable().optional(),
  parents: z.boolean().nullable().optional(),
  files: z.array(writeFileSpecSchema).nullable().optional(),
})
export type FsWriteRequest = z.infer<typeof fsWriteRequestSchema>

export const writeFileResultSchema = z.object({
  path: z.string(),
  bytes_written: z.number(),
})
export type WriteFileResult = z.infer<typeof writeFileResultSchema>

export const fsWriteResponseSchema = z.object({
  bytes_written: z.number(),
  path: z.string(),
  files: z.array(writeFileResultSchema).optional().default([]),
})
export type FsWriteResponse = z.infer<typeof fsWriteResponseSchema>

/** `ReadResponseWire` — `content` is ALWAYS a channel ref, never inline
    (the handler converts before serializing). */
export const fsReadResponseSchema = z.object({
  content: contentRefSchema,
  size: z.number(),
  mode: z.string(),
  mtime: z.number(),
})
export type FsReadResponse = z.infer<typeof fsReadResponseSchema>

// ---------------------------------------------------------------------------
// Errors — flat SDK ErrorBody `{ code, message, stacktrace? }` plus the
// harness re-wrap that embeds the S-code into a denial-reason string.
// ---------------------------------------------------------------------------

/** The SDK `ErrorBody` carrying a typed shell S-code (`IIIError::Remote`).
    No `type` field — the generic wire schema in ../lib/errors requires
    one, so shell errors need this dedicated schema. The regex keeps it
    from false-positiving on arbitrary `{ code, message }` objects. */
export const shellErrorWireSchema = z.object({
  code: z.string().regex(/^S\d{3}$/),
  message: z.string(),
  stacktrace: z.string().optional(),
})
export type ShellErrorWire = z.infer<typeof shellErrorWireSchema>

/** Human labels for the S-codes documented in `shell/src/main.rs`. */
export const S_CODE_LABEL: Record<string, string> = {
  S200: 'in-VM failure',
  S210: 'bad request',
  S211: 'not found',
  S212: 'wrong file type',
  S213: 'already exists',
  S214: 'directory not empty',
  S215: 'filesystem access blocked',
  S216: 'i/o error',
  S217: 'bad regex',
  S218: 'size cap exceeded',
  S300: 'vm boot failed',
}

/** Lift a flat shell error into the shared `ErrorDisplay` wire variant so
    `ErrorDisplayView` renders it unchanged (S-code Badge + label +
    message). */
function shellWire(e: { code: string; message: string }): ErrorDisplay {
  return {
    variant: 'wire',
    error: {
      type: S_CODE_LABEL[e.code] ?? 'shell error',
      code: e.code,
      message: cleanShellErrorMessage(e.message),
    },
  }
}

export function cleanShellErrorMessage(message: string): string {
  return stripHandlerPrefix(message)
    .replace(/^remote error \(S\d{3}\):\s*/, '')
    .replace(/\s+filesystem_access_request=\{[\s\S]*\}\s*$/, '')
    .trim()
}

/** exec_bg stringifies its S-code into the message ("S215: …"), and the
    SDK Display impl prefixes "handler error: " / "serialization error: ".
    Pull the embedded S-code out of free text. */
export function extractEmbeddedSCode(message: string): string | null {
  return message.match(/\b(S\d{3}):/)?.[1] ?? null
}

/** Strip the harness/SDK Display prefixes ("trigger_failed: <Class>: ",
    "handler error: ", "serialization error: ") for the headline — the
    raw text stays available in the raw-json tab. */
export function stripHandlerPrefix(message: string): string {
  return message
    .replace(/^trigger_failed:.*?:\s*/, '')
    .replace(/^(?:handler error|serialization error):\s*/, '')
}

/** Synthesize a wire display from an S-code embedded in free text —
    `"trigger_failed: IIIInvocationError: S211: no such job: job-x"` →
    `{ code: 'S211', message: 'no such job: job-x' }`. */
function wireFromText(text: string): ErrorDisplay | null {
  const match = text.match(/\b(S\d{3}):\s*([\s\S]+)/)
  if (!match) return null
  return shellWire({
    code: match[1],
    message: cleanShellErrorMessage(match[2].trim()),
  })
}

/** Every string the harness might hide an S-code in: string candidates
    from the shared traversal, plus `reason`/`message` fields of object
    candidates (the gate-denial envelope keeps its prose in `reason`,
    which the generic traversal surfaces only as an object). */
function collectStringCandidates(value: unknown): string[] {
  const seen = new Set<string>()
  const out: string[] = []
  const push = (candidate: unknown) => {
    if (typeof candidate !== 'string' || candidate.length === 0) return
    if (seen.has(candidate)) return
    seen.add(candidate)
    out.push(candidate)
  }

  for (const candidate of collectErrorCandidates(value)) {
    for (const layer of [candidate, unwrapEnvelope(candidate)]) {
      push(layer)
      if (layer && typeof layer === 'object' && !Array.isArray(layer)) {
        const obj = layer as Record<string, unknown>
        push(obj.reason)
        push(obj.message)
      }
    }
  }

  return out
}

/**
 * Normalise every failure shape the console may receive for shell calls,
 * in priority order:
 *
 * 1. Flat SDK `ErrorBody` with a typed S-code — direct objects anywhere
 *    in the candidate traversal, or JSON embedded in strings.
 * 2. S-codes stringified into prose (the load-bearing real-world path):
 *    the harness re-wraps shell's plain-text errors into a gate-denial
 *    envelope whose `reason` reads e.g. `"trigger_failed:
 *    IIIInvocationError: S211: no such job: job-x"` — scan every string
 *    candidate and synthesize the wire display from text.
 * 3. Delegate to `parseErrorDisplay` for denials without S-codes,
 *    generic function_error envelopes, and harness wrappers.
 *
 * Returns the shared `ErrorDisplay` union, rendered by `ErrorDisplayView`
 * unchanged.
 */
export function parseShellErrorDisplay(value: unknown): ErrorDisplay | null {
  const candidates = collectErrorCandidates(value)

  for (const candidate of candidates) {
    const direct = shellErrorWireSchema.safeParse(candidate)
    if (direct.success) return shellWire(direct.data)

    if (typeof candidate === 'string') {
      const embedded = extractFirstJsonObject(candidate)
      if (embedded != null) {
        const fromJson = shellErrorWireSchema.safeParse(embedded)
        if (fromJson.success) return shellWire(fromJson.data)
      }
    }
  }

  for (const text of collectStringCandidates(value)) {
    const synthesized = wireFromText(text)
    if (synthesized) return synthesized
  }

  return parseErrorDisplay(value)
}
