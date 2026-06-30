import { z } from 'zod'

/* Zod schemas mirroring the sandbox::* request/response shapes from
   `motia/crates/iii-worker/src/sandbox_daemon/*` and the shared
   `iii-shell-proto` (FsEntry, FsMatch, FsSedFileResult).

   The Rust handlers route via `RegisterFunction::new_async` with
   `JsonSchema` deserialisers, so wire payloads are always plain JSON
   matching these shapes. Every schema is intentionally non-strict
   (no `.strict()`); forward-compat is preserved for unknown future
   fields and the renderers only read what they need. */

// ---------------------------------------------------------------------------
// Shared building blocks
// ---------------------------------------------------------------------------

/** `engine/src/protocol.rs::StreamChannelRef` (untagged JSON object). */
export const streamChannelRefSchema = z.object({
  channel_id: z.string(),
  access_key: z.string(),
  direction: z.enum(['read', 'write']),
})
export type StreamChannelRef = z.infer<typeof streamChannelRefSchema>

/** Either an inline UTF-8 body or a streaming channel ref. */
export const fileContentSchema = z.union([z.string(), streamChannelRefSchema])
export type FileContent = z.infer<typeof fileContentSchema>

/** `iii-shell-proto::FsEntry`. */
export const fsEntrySchema = z.object({
  name: z.string(),
  is_dir: z.boolean(),
  size: z.number(),
  mode: z.string(),
  mtime: z.number(),
  is_symlink: z.boolean(),
})
export type FsEntry = z.infer<typeof fsEntrySchema>

/** `iii-shell-proto::FsMatch`. `path` is canonical; older guests sent
    `file` — `aliasPath` peels that legacy spelling. */
export const fsMatchSchema = z
  .object({
    path: z.string().optional(),
    file: z.string().optional(),
    line: z.number(),
    content: z.string(),
  })
  .transform((m) => ({
    path: m.path ?? m.file ?? '',
    line: m.line,
    content: m.content,
  }))
export type FsMatch = z.infer<typeof fsMatchSchema>

/** `iii-shell-proto::FsSedFileResult`. */
export const fsSedFileResultSchema = z
  .object({
    path: z.string().optional(),
    file: z.string().optional(),
    replacements: z.number(),
    success: z.boolean(),
    error: z.string().nullable().optional(),
  })
  .transform((r) => ({
    path: r.path ?? r.file ?? '',
    replacements: r.replacements,
    success: r.success,
    error: r.error ?? null,
  }))
export type FsSedFileResult = z.infer<typeof fsSedFileResultSchema>

/** EnvShape from exec.rs — accepts either Vec<"K=V"> or {K:V}. */
export const envShapeSchema = z.union([
  z.array(z.string()),
  z.record(z.string(), z.string()),
])
export type EnvShape = z.infer<typeof envShapeSchema>

// ---------------------------------------------------------------------------
// SandboxErrorWire — the Stripe-style flat error payload from errors.rs.
// ---------------------------------------------------------------------------

export const sandboxErrorWireSchema = z.object({
  type: z.string(),
  code: z.string().regex(/^S\d{3}$/),
  message: z.string(),
  docs_url: z.string().optional(),
  retryable: z.boolean().optional(),
  fix: z.unknown().optional(),
  fix_note: z.string().nullable().optional(),
})
export type SandboxErrorWire = z.infer<typeof sandboxErrorWireSchema>

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

export const execRequestSchema = z.object({
  sandbox_id: z.string(),
  cmd: z.string().optional(),
  args: z.array(z.string()).optional(),
  argv: z.array(z.string()).optional(),
  stdin: z.string().nullable().optional(),
  env: envShapeSchema.optional(),
  timeout_ms: z.number().nullable().optional(),
  workdir: z.string().nullable().optional(),
})
export type ExecRequest = z.infer<typeof execRequestSchema>

export const execResponseSchema = z.object({
  stdout: z.string(),
  stderr: z.string(),
  exit_code: z.number().nullable(),
  timed_out: z.boolean(),
  duration_ms: z.number(),
  success: z.boolean(),
})
export type ExecResponse = z.infer<typeof execResponseSchema>

export const runFileSchema = z.object({
  path: z.string(),
  content: z.string(),
})

export const runRequestSchema = z.object({
  image: z.string(),
  code: z.string(),
  lang: z.string(),
  files: z.array(runFileSchema).optional(),
  env: envShapeSchema.optional(),
  stdin: z.string().nullable().optional(),
  timeout_ms: z.number().nullable().optional(),
  keep_sandbox: z.boolean().optional(),
})
export type RunRequest = z.infer<typeof runRequestSchema>

export const runResponseSchema = z.object({
  stdout: z.string(),
  stderr: z.string(),
  exit_code: z.number().nullable(),
  timed_out: z.boolean(),
  duration_ms: z.number(),
  success: z.boolean(),
  sandbox_id: z.string().nullable().optional(),
})
export type RunResponse = z.infer<typeof runResponseSchema>

export const createRequestSchema = z.object({
  image: z.string(),
  cpus: z.number().nullable().optional(),
  memory_mb: z.number().nullable().optional(),
  name: z.string().nullable().optional(),
  network: z.boolean().nullable().optional(),
  idle_timeout_secs: z.number().nullable().optional(),
  env: envShapeSchema.optional(),
})
export type CreateRequest = z.infer<typeof createRequestSchema>

export const createResponseSchema = z.object({
  sandbox_id: z.string(),
  image: z.string(),
})
export type CreateResponse = z.infer<typeof createResponseSchema>

export const stopRequestSchema = z.object({
  sandbox_id: z.string(),
  wait: z.boolean().optional(),
})
export type StopRequest = z.infer<typeof stopRequestSchema>

export const stopResponseSchema = z.object({
  sandbox_id: z.string(),
  stopped: z.boolean(),
})
export type StopResponse = z.infer<typeof stopResponseSchema>

export const listRequestSchema = z.object({}).passthrough()
export type ListRequest = z.infer<typeof listRequestSchema>

export const sandboxSummarySchema = z.object({
  sandbox_id: z.string(),
  name: z.string().nullable().optional(),
  image: z.string(),
  age_secs: z.number(),
  exec_in_progress: z.boolean(),
  stopped: z.boolean(),
})
export type SandboxSummary = z.infer<typeof sandboxSummarySchema>

export const listResponseSchema = z.object({
  sandboxes: z.array(sandboxSummarySchema),
})
export type ListResponse = z.infer<typeof listResponseSchema>

// ---------------------------------------------------------------------------
// Filesystem
// ---------------------------------------------------------------------------

export const fsLsRequestSchema = z.object({
  sandbox_id: z.string(),
  path: z.string(),
})
export type FsLsRequest = z.infer<typeof fsLsRequestSchema>

export const fsLsResponseSchema = z.object({
  entries: z.array(fsEntrySchema),
})
export type FsLsResponse = z.infer<typeof fsLsResponseSchema>

export const fsStatRequestSchema = z.object({
  sandbox_id: z.string(),
  path: z.string(),
})
export type FsStatRequest = z.infer<typeof fsStatRequestSchema>

export const fsStatResponseSchema = z.object({
  name: z.string(),
  is_dir: z.boolean(),
  size: z.number(),
  mode: z.string(),
  mtime: z.number(),
  is_symlink: z.boolean(),
})
export type FsStatResponse = z.infer<typeof fsStatResponseSchema>

export const fsMkdirRequestSchema = z.object({
  sandbox_id: z.string(),
  path: z.string(),
  mode: z.string().optional(),
  parents: z.boolean().optional(),
})
export type FsMkdirRequest = z.infer<typeof fsMkdirRequestSchema>

export const fsMkdirResponseSchema = z.object({
  created: z.boolean(),
})
export type FsMkdirResponse = z.infer<typeof fsMkdirResponseSchema>

export const fsWriteRequestSchema = z.object({
  sandbox_id: z.string(),
  path: z.string(),
  mode: z.string().optional(),
  parents: z.boolean().optional(),
  content: fileContentSchema.nullable().optional(),
  content_b64: z.string().nullable().optional(),
})
export type FsWriteRequest = z.infer<typeof fsWriteRequestSchema>

export const fsWriteResponseSchema = z.object({
  bytes_written: z.number(),
  path: z.string(),
})
export type FsWriteResponse = z.infer<typeof fsWriteResponseSchema>

export const fsReadRequestSchema = z.object({
  sandbox_id: z.string(),
  path: z.string(),
})
export type FsReadRequest = z.infer<typeof fsReadRequestSchema>

export const fsReadResponseSchema = z.object({
  content: fileContentSchema,
  size: z.number(),
  mode: z.string(),
  mtime: z.number(),
})
export type FsReadResponse = z.infer<typeof fsReadResponseSchema>

export const fsRmRequestSchema = z.object({
  sandbox_id: z.string(),
  path: z.string(),
  recursive: z.boolean().optional(),
})
export type FsRmRequest = z.infer<typeof fsRmRequestSchema>

export const fsRmResponseSchema = z.object({
  removed: z.boolean(),
})
export type FsRmResponse = z.infer<typeof fsRmResponseSchema>

export const fsMvRequestSchema = z.object({
  sandbox_id: z.string(),
  src: z.string(),
  dst: z.string(),
  overwrite: z.boolean().optional(),
})
export type FsMvRequest = z.infer<typeof fsMvRequestSchema>

export const fsMvResponseSchema = z.object({
  moved: z.boolean(),
})
export type FsMvResponse = z.infer<typeof fsMvResponseSchema>

export const fsChmodRequestSchema = z.object({
  sandbox_id: z.string(),
  path: z.string(),
  mode: z.string(),
  uid: z.number().nullable().optional(),
  gid: z.number().nullable().optional(),
  recursive: z.boolean().optional(),
})
export type FsChmodRequest = z.infer<typeof fsChmodRequestSchema>

export const fsChmodResponseSchema = z.object({
  updated: z.number(),
})
export type FsChmodResponse = z.infer<typeof fsChmodResponseSchema>

export const fsGrepRequestSchema = z.object({
  sandbox_id: z.string(),
  path: z.string(),
  pattern: z.string(),
  recursive: z.boolean().optional(),
  ignore_case: z.boolean().optional(),
  include_glob: z.array(z.string()).optional(),
  exclude_glob: z.array(z.string()).optional(),
  max_matches: z.number().optional(),
  max_line_bytes: z.number().optional(),
})
export type FsGrepRequest = z.infer<typeof fsGrepRequestSchema>

export const fsGrepResponseSchema = z.object({
  matches: z.array(fsMatchSchema),
  truncated: z.boolean(),
})
export type FsGrepResponse = z.infer<typeof fsGrepResponseSchema>

export const fsSedRequestSchema = z.object({
  sandbox_id: z.string(),
  files: z.array(z.string()).optional(),
  path: z.string().nullable().optional(),
  recursive: z.boolean().optional(),
  include_glob: z.array(z.string()).optional(),
  exclude_glob: z.array(z.string()).optional(),
  pattern: z.string(),
  replacement: z.string(),
  regex: z.boolean().optional(),
  first_only: z.boolean().optional(),
  ignore_case: z.boolean().optional(),
})
export type FsSedRequest = z.infer<typeof fsSedRequestSchema>

export const fsSedResponseSchema = z.object({
  results: z.array(fsSedFileResultSchema),
  total_replacements: z.number(),
})
export type FsSedResponse = z.infer<typeof fsSedResponseSchema>

// ---------------------------------------------------------------------------
// Envelope unwrap
// ---------------------------------------------------------------------------

/**
 * The harness `agent-trigger.ts` wraps every tool result in a
 * `{ content: ContentBlock[], details: unknown, terminate: boolean }`
 * envelope before relaying it to the agent. The console receives the
 * same shape via the engine's function_call output stream.
 *
 * This peels the wrapper so renderers operate on the flat sandbox
 * response. Idempotent — calling it on an already-flat payload
 * (i.e. one that didn't go through the envelope) returns the input
 * unchanged. The discriminator is `Array.isArray(value.content)` —
 * that's the structural marker harness sets unconditionally for
 * every wrapped result.
 */
export function unwrapEnvelope(value: unknown): unknown {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return value
  }
  const obj = value as Record<string, unknown>
  if (Array.isArray(obj.content) && 'details' in obj) {
    return obj.details
  }
  return value
}

const denialEnvelopeSchema = z.object({
  schema_version: z.number().optional(),
  status: z.string().optional(),
  denied_by: z.string().optional(),
  function_id: z.string().optional(),
  reason: z.string().optional(),
})
export type DenialEnvelopeWire = z.infer<typeof denialEnvelopeSchema>

const functionErrorEnvelopeSchema = z.object({
  kind: z.string(),
  message: z.string(),
  details: z.unknown().optional(),
  content: z.array(z.unknown()).optional(),
})

export type SandboxInvocationError = {
  title: string
  message: string
  functionId?: string
  deniedBy?: string
  reason?: string
  detailText?: string
}

/**
 * A fail-closed dispatch-policy denial: the calling agent's `functions`
 * allow-list does not cover the function it tried. Distinct from a permission
 * denial (approval gate) — this is structural and the agent cannot retry it.
 */
export interface SandboxDispatchDenial {
  /** The blocked function id, when the message names it (`function <id> …`). */
  functionId?: string
  /** Namespace prefix of `functionId` (e.g. `web` from `web::fetch`). */
  namespace?: string
  /** The raw engine message, kept verbatim under the actionable hint. */
  message: string
}

export type SandboxErrorDisplay =
  | { variant: 'wire'; error: SandboxErrorWire }
  | { variant: 'invocation'; error: SandboxInvocationError }
  | { variant: 'dispatch-denied'; error: SandboxDispatchDenial }

function contentBlocksText(content: unknown): string | undefined {
  if (!Array.isArray(content)) return undefined
  const parts: string[] = []
  for (const block of content) {
    if (!block || typeof block !== 'object') continue
    const obj = block as Record<string, unknown>
    if (
      obj.type === 'text' &&
      typeof obj.text === 'string' &&
      obj.text.length > 0
    ) {
      parts.push(obj.text)
    }
  }
  return parts.length > 0 ? parts.join('\n') : undefined
}

/** Pull the first balanced `{…}` JSON object out of a string. */
export function extractFirstJsonObject(text: string): unknown | null {
  const start = text.indexOf('{')
  if (start === -1) return null
  let depth = 0
  for (let i = start; i < text.length; i++) {
    const ch = text[i]
    if (ch === '{') depth++
    else if (ch === '}') {
      depth--
      if (depth === 0) {
        try {
          return JSON.parse(text.slice(start, i + 1))
        } catch {
          return null
        }
      }
    }
  }
  return null
}

function tryParseWire(value: unknown): SandboxErrorWire | null {
  const parsed = sandboxErrorWireSchema.safeParse(unwrapEnvelope(value))
  if (parsed.success) return parsed.data

  if (!value || typeof value !== 'object' || Array.isArray(value)) return null
  const obj = value as Record<string, unknown>
  if (
    obj.error === 'handler_error' ||
    (typeof obj.code === 'string' && /^S\d{3}$/.test(obj.code))
  ) {
    const { error: _tag, ...rest } = obj
    const tagged = sandboxErrorWireSchema.safeParse(rest)
    if (tagged.success) return tagged.data
  }
  return null
}

function tryParseDenial(value: unknown): DenialEnvelopeWire | null {
  const parsed = denialEnvelopeSchema.safeParse(unwrapEnvelope(value))
  if (!parsed.success) return null
  if (parsed.data.status !== 'denied' && !parsed.data.denied_by) return null
  return parsed.data
}

function denialToInvocation(
  denial: DenialEnvelopeWire,
  fallbackMessage?: string,
  detailText?: string,
): SandboxInvocationError {
  const deniedBy = denial.denied_by
  const title =
    deniedBy === 'gate_unavailable'
      ? 'Gate unavailable'
      : deniedBy === 'permissions'
        ? 'Permission denied'
        : deniedBy === 'user'
          ? 'Denied by user'
          : denial.status === 'denied'
            ? 'Denied'
            : 'Invocation failed'
  const message =
    denial.reason ?? fallbackMessage ?? 'The sandbox call could not complete.'
  return {
    title,
    message,
    functionId: denial.function_id,
    deniedBy,
    reason: denial.reason,
    detailText,
  }
}

function invocationFromFunctionError(
  envelope: z.infer<typeof functionErrorEnvelopeSchema>,
): SandboxInvocationError | null {
  const detailText = contentBlocksText(envelope.content)
  const denial =
    envelope.details != null ? tryParseDenial(envelope.details) : null
  if (denial) {
    return denialToInvocation(denial, envelope.message, detailText)
  }
  return {
    title: 'Invocation failed',
    message: envelope.message,
    detailText,
  }
}

/** Gather every sub-value that might carry an error payload: the value
    itself, its unwrapped envelope, `value.error`, `error.details`,
    `error.message`, and `content[]` text blocks. Shared with the shell
    module's `parseShellErrorDisplay` — same harness wrapping, same
    traversal. */
export function collectErrorCandidates(value: unknown): unknown[] {
  const seen = new Set<unknown>()
  const out: unknown[] = []
  const push = (candidate: unknown) => {
    if (seen.has(candidate)) return
    seen.add(candidate)
    out.push(candidate)
  }

  push(value)
  push(unwrapEnvelope(value))

  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const obj = value as Record<string, unknown>
    if (obj.error && typeof obj.error === 'object') {
      const err = obj.error as Record<string, unknown>
      push(err)
      if ('details' in err) push(err.details)
      if (typeof err.message === 'string') push(err.message)
      const text = contentBlocksText(err.content)
      if (text) push(text)
    }
  }

  return out
}

/** Engine fail-closed dispatch-policy denial signature (harness policy.rs). */
const DISPATCH_DENIAL_RE = /not permitted by this agent.?s dispatch policy/i
/** `function <id> is not permitted …` — captures the blocked function id. */
const DISPATCH_FN_RE = /function\s+([A-Za-z0-9_:-]+)\s+is not permitted/i

/** A message-like string from a candidate, for signature scanning. */
function candidateText(candidate: unknown): string | undefined {
  if (typeof candidate === 'string') return candidate
  if (candidate && typeof candidate === 'object' && !Array.isArray(candidate)) {
    const msg = (candidate as Record<string, unknown>).message
    if (typeof msg === 'string') return msg
  }
  return undefined
}

/**
 * Detect a fail-closed dispatch-policy denial in any failure shape. Scans the
 * same candidate set as `parseSandboxErrorDisplay` for the engine signature and
 * pulls the blocked function id out of the message when present.
 */
export function parseDispatchDenial(
  value: unknown,
): SandboxDispatchDenial | null {
  for (const candidate of collectErrorCandidates(value)) {
    const text = candidateText(candidate)
    if (!text || !DISPATCH_DENIAL_RE.test(text)) continue
    const functionId = text.match(DISPATCH_FN_RE)?.[1]
    const namespace = functionId?.includes('::')
      ? functionId.split('::')[0]
      : undefined
    return { functionId, namespace, message: text }
  }
  return null
}

/**
 * Normalise every failure shape the console may receive for sandbox calls:
 * flat `SandboxErrorWire`, harness `{ content, details }` wrappers, the
 * translate `{ error: { kind, message, details, content } }` envelope,
 * handler-tagged payloads, denial envelopes, and JSON embedded in strings.
 */
export function parseSandboxErrorDisplay(
  value: unknown,
): SandboxErrorDisplay | null {
  const candidates = collectErrorCandidates(value)

  // Dispatch-policy denial is the most specific + actionable shape — detect it
  // before the generic invocation-failed fallback swallows it as raw text.
  const dispatchDenial = parseDispatchDenial(value)
  if (dispatchDenial) {
    return { variant: 'dispatch-denied', error: dispatchDenial }
  }

  for (const candidate of candidates) {
    const wire = tryParseWire(candidate)
    if (wire) return { variant: 'wire', error: wire }

    if (typeof candidate === 'string') {
      const embedded = extractFirstJsonObject(candidate)
      if (embedded != null) {
        const wireFromText = tryParseWire(embedded)
        if (wireFromText) return { variant: 'wire', error: wireFromText }
      }
    }
  }

  for (const candidate of candidates) {
    const denial = tryParseDenial(candidate)
    if (denial) {
      return {
        variant: 'invocation',
        error: denialToInvocation(denial),
      }
    }
  }

  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const parsed = functionErrorEnvelopeSchema.safeParse(
      (value as Record<string, unknown>).error,
    )
    if (parsed.success && parsed.data.kind === 'function_error') {
      const invocation = invocationFromFunctionError(parsed.data)
      if (invocation) {
        return { variant: 'invocation', error: invocation }
      }
    }
  }

  return null
}

/** Returns true if a value looks like a `SandboxErrorWire` payload. */
export function isSandboxErrorWire(value: unknown): value is SandboxErrorWire {
  return tryParseWire(value) != null
}

export const SANDBOX_FUNCTION_IDS = [
  'sandbox::create',
  'sandbox::exec',
  'sandbox::stop',
  'sandbox::list',
  'sandbox::run',
  'sandbox::fs::ls',
  'sandbox::fs::stat',
  'sandbox::fs::mkdir',
  'sandbox::fs::write',
  'sandbox::fs::read',
  'sandbox::fs::rm',
  'sandbox::fs::chmod',
  'sandbox::fs::mv',
  'sandbox::fs::grep',
  'sandbox::fs::sed',
] as const

export type SandboxFunctionId = (typeof SANDBOX_FUNCTION_IDS)[number]

export function isSandboxFunctionId(id: string): id is SandboxFunctionId {
  return (SANDBOX_FUNCTION_IDS as readonly string[]).includes(id)
}

export function parseSandboxError(value: unknown): SandboxErrorWire | null {
  return tryParseWire(value)
}

export function safeParseRequest<T>(
  schema: z.ZodType<T>,
  value: unknown,
): T | null {
  const parsed = schema.safeParse(value ?? {})
  return parsed.success ? parsed.data : null
}

export function safeParseResponse<T>(
  schema: z.ZodType<T>,
  value: unknown,
): T | null {
  const parsed = schema.safeParse(unwrapEnvelope(value))
  return parsed.success ? parsed.data : null
}
