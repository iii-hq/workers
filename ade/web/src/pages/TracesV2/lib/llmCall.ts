/**
 * The LLM call behind a span, read for the span panel's model view.
 *
 * Two sources, merged:
 *   - OTel `gen_ai.*` attributes on the span (model, system/provider, finish
 *     reason, token usage) — the semantic-conventions shape any instrumented
 *     client emits;
 *   - the iii-sdk auto-captured request/response (`iii.payload.json` events,
 *     surfaced through `functionTriggerFromSpan`) for the llm-router and
 *     provider workers: `{ model, provider, system_prompt, messages[],
 *     tools[], max_output_tokens, … }` in, `{ model, provider, stop_reason,
 *     usage, ok }` out.
 *
 * The engine caps a captured payload at 64 KB and marks the cut with a
 * trailing `…[truncated N bytes]`; the request of a real chat turn is
 * almost always past that cap, so the raw string no longer parses. Rather
 * than fall back to one 65 000-character line, `repairTruncatedJson` closes
 * the open strings and brackets and the view says how much was dropped.
 */

export interface LlmUsage {
  input?: number
  output?: number
  cacheRead?: number
  cacheWrite?: number
  costUsd?: number
}

export interface LlmContentBlock {
  type: string
  /** `text` / `thinking` blocks */
  text?: string
  /** `tool_use` / `function_call` blocks */
  name?: string
  id?: string
  input?: unknown
  /** `tool_result` blocks */
  toolUseId?: string
  isError?: boolean
  /** anything the block carried that isn't modelled above */
  raw: unknown
}

export interface LlmMessage {
  role: string
  blocks: LlmContentBlock[]
  /** an assistant turn recorded with its own model/usage (router history) */
  model?: string
  stopReason?: string
  usage?: LlmUsage
  timestamp?: number
}

export interface LlmTool {
  name: string
  description?: string
  parameters?: unknown
}

export interface LlmCall {
  model?: string
  provider?: string
  stopReason?: string
  usage?: LlmUsage
  systemPrompt?: string
  messages: LlmMessage[]
  tools: LlmTool[]
  /** request fields that aren't messages/tools/system (max tokens, temperature…) */
  params: Record<string, unknown>
  /** the response minus `usage`/`model`/`provider` bookkeeping */
  responseFields: Record<string, unknown>
  /** assistant content in the response, when the worker returned any */
  responseBlocks: LlmContentBlock[]
  /** bytes the engine dropped from the captured request, if it was cut */
  requestTruncatedBytes: number | null
  responseTruncatedBytes: number | null
  /** the request/response as parsed (repaired when truncated) */
  request: unknown
  response: unknown
}

const TRUNCATION_MARK = /…\[truncated (\d+) bytes\]\s*$/

/** The byte count a payload's trailing truncation marker claims, or null. */
export function truncatedBytesOf(raw: unknown): number | null {
  if (typeof raw !== 'string') return null
  const m = TRUNCATION_MARK.exec(raw)
  return m ? Number.parseInt(m[1], 10) : null
}

/**
 * Close whatever a cut-off JSON document left open so it parses again.
 *
 * Scans once, tracking string state and the bracket stack. The cut lands in
 * one of a handful of places — inside a string, after a key's colon, after a
 * comma, mid-number, or mid-literal — and each is patched at the tail:
 * unterminated strings are closed, a dangling `:` gets `null`, a dangling
 * `,` or an unfinished key/literal is dropped back to the last complete
 * structural position, then the stack is unwound. Returns null when no
 * repair parses (a document that was never JSON).
 */
export function repairTruncatedJson(raw: string): unknown | null {
  let text = raw.replace(TRUNCATION_MARK, '')
  // Already whole? Then there's nothing to repair.
  try {
    return JSON.parse(text)
  } catch {
    /* fall through to the repair */
  }
  const trimmed = text.trimStart()
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) return null
  text = trimmed

  // One pass: bracket stack plus the index after each structural comma or
  // opening bracket (a position the document can be cut back to cleanly).
  const stack: string[] = []
  const safeCuts: number[] = []
  let inString = false
  let escaped = false
  for (let i = 0; i < text.length; i++) {
    const ch = text[i]
    if (inString) {
      if (escaped) escaped = false
      else if (ch === '\\') escaped = true
      else if (ch === '"') inString = false
      continue
    }
    if (ch === '"') inString = true
    else if (ch === '{' || ch === '[') {
      stack.push(ch)
      safeCuts.push(i + 1)
    } else if (ch === '}' || ch === ']') stack.pop()
    else if (ch === ',') safeCuts.push(i + 1)
  }

  const closeAll = (body: string, depth: string[]): string => {
    let out = body
    for (let i = depth.length - 1; i >= 0; i--) {
      out += depth[i] === '{' ? '}' : ']'
    }
    return out
  }
  const tryParse = (candidate: string): unknown | null => {
    try {
      return JSON.parse(candidate)
    } catch {
      return null
    }
  }

  // 1. Cut inside a string: close it (dropping a trailing lone backslash so
  //    the closing quote isn't escaped), then patch the tail like any other.
  let body = text
  if (inString) {
    if (escaped) body = body.slice(0, -1)
    body = `${body}"`
  }
  const tail = (s: string) => s.replace(/\s+$/, '')
  // 2. Dangling `:` → the value was lost; `null` keeps the key visible.
  //    Dangling `,` → drop it. Both leave the stack as computed.
  let patched = tail(body)
  if (patched.endsWith(':')) patched = `${patched}null`
  else if (patched.endsWith(',')) patched = patched.slice(0, -1)
  let parsed = tryParse(closeAll(patched, stack))
  if (parsed !== null) return parsed

  // 3. A half-written key, number or literal (`tru`, `"ke`): fall back to
  //    the last structural position, recomputing the stack for that prefix.
  for (let n = safeCuts.length - 1; n >= 0 && n >= safeCuts.length - 8; n--) {
    const cut = safeCuts[n]
    const prefix = tail(text.slice(0, cut))
    const depth: string[] = []
    let inStr = false
    let esc = false
    for (let i = 0; i < prefix.length; i++) {
      const ch = prefix[i]
      if (inStr) {
        if (esc) esc = false
        else if (ch === '\\') esc = true
        else if (ch === '"') inStr = false
        continue
      }
      if (ch === '"') inStr = true
      else if (ch === '{' || ch === '[') depth.push(ch)
      else if (ch === '}' || ch === ']') depth.pop()
    }
    const cleaned = prefix.endsWith(',') ? prefix.slice(0, -1) : prefix
    parsed = tryParse(closeAll(cleaned, depth))
    if (parsed !== null) return parsed
  }
  return null
}

/** Parse a captured payload: an object passes through, a JSON string parses,
 *  a truncated JSON string is repaired. Anything else returns as-is. */
export function parsePayload(raw: unknown): unknown {
  if (typeof raw !== 'string') return raw
  const trimmed = raw.trim()
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) return raw
  try {
    return JSON.parse(trimmed)
  } catch {
    return repairTruncatedJson(trimmed) ?? raw
  }
}

const isRecord = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === 'object' && !Array.isArray(v)

const num = (v: unknown): number | undefined =>
  typeof v === 'number' && Number.isFinite(v)
    ? v
    : typeof v === 'string' && v.trim() !== '' && Number.isFinite(Number(v))
      ? Number(v)
      : undefined

const str = (v: unknown): string | undefined =>
  typeof v === 'string' && v.length > 0 ? v : undefined

function usageOf(v: unknown): LlmUsage | undefined {
  if (!isRecord(v)) return undefined
  const usage: LlmUsage = {
    input: num(v.input ?? v.input_tokens ?? v.prompt_tokens),
    output: num(v.output ?? v.output_tokens ?? v.completion_tokens),
    cacheRead: num(v.cache_read ?? v.cache_read_input_tokens),
    cacheWrite: num(v.cache_write ?? v.cache_creation_input_tokens),
    costUsd: num(v.cost_usd),
  }
  return Object.values(usage).some((n) => n !== undefined) ? usage : undefined
}

function blockOf(v: unknown): LlmContentBlock {
  if (typeof v === 'string') return { type: 'text', text: v, raw: v }
  if (!isRecord(v)) return { type: 'unknown', raw: v }
  const type = str(v.type) ?? (str(v.text) ? 'text' : 'unknown')
  return {
    type,
    text: str(v.text) ?? str(v.thinking),
    name: str(v.name) ?? str((v.function as Record<string, unknown>)?.name),
    id: str(v.id),
    input:
      v.input ??
      v.arguments ??
      (isRecord(v.function) ? v.function.arguments : undefined),
    toolUseId: str(v.tool_use_id) ?? str(v.tool_call_id),
    isError: typeof v.is_error === 'boolean' ? v.is_error : undefined,
    raw: v,
  }
}

function blocksOf(content: unknown): LlmContentBlock[] {
  if (content === undefined || content === null) return []
  if (Array.isArray(content)) return content.map(blockOf)
  return [blockOf(content)]
}

function messageOf(v: unknown): LlmMessage | null {
  if (!isRecord(v)) return null
  const role = str(v.role) ?? 'unknown'
  return {
    role,
    blocks: blocksOf(v.content ?? v.text),
    model: str(v.model),
    stopReason: str(v.stop_reason) ?? str(v.finish_reason),
    usage: usageOf(v.usage),
    timestamp: num(v.timestamp),
  }
}

function toolOf(v: unknown): LlmTool | null {
  if (!isRecord(v)) return null
  const fn = isRecord(v.function) ? v.function : v
  const name = str(fn.name)
  if (!name) return null
  return {
    name,
    description: str(fn.description),
    parameters: fn.parameters ?? fn.input_schema,
  }
}

const REQUEST_HANDLED = new Set([
  'messages',
  'tools',
  'system_prompt',
  'system',
  'model',
  'provider',
  '_caller_worker_id',
])
const RESPONSE_HANDLED = new Set([
  'usage',
  'model',
  'provider',
  'stop_reason',
  'finish_reason',
  'content',
  'message',
  'ok',
])

/** Does a span's function/attributes/payload describe an LLM call at all? */
export function isLlmCall(
  functionId: string | null,
  attributes: Record<string, unknown> | undefined,
  input: unknown,
): boolean {
  if (
    attributes &&
    Object.keys(attributes).some((k) => k.startsWith('gen_ai.'))
  )
    return true
  if (functionId && /^router::(chat|count_tokens|stream)$/.test(functionId))
    return true
  if (
    functionId &&
    /^provider::[^:]+::(stream|chat|complete|count_tokens)$/.test(functionId)
  )
    return true
  const parsed = parsePayload(input)
  return isRecord(parsed) && Array.isArray(parsed.messages) && 'model' in parsed
}

/** Read the LLM call out of a span's attributes and captured payloads. */
export function llmCallFromSpan(
  attributes: Record<string, unknown> | undefined,
  rawInput: unknown,
  rawOutput: unknown,
): LlmCall {
  const attrs = attributes ?? {}
  const request = parsePayload(rawInput)
  const response = parsePayload(rawOutput)
  const req = isRecord(request) ? request : {}
  const res = isRecord(response) ? response : {}

  const messages = Array.isArray(req.messages)
    ? req.messages.map(messageOf).filter((m): m is LlmMessage => m !== null)
    : []
  const tools = Array.isArray(req.tools)
    ? req.tools.map(toolOf).filter((t): t is LlmTool => t !== null)
    : []
  const systemPrompt =
    str(req.system_prompt) ??
    (typeof req.system === 'string'
      ? req.system
      : Array.isArray(req.system)
        ? blocksOf(req.system)
            .map((b) => b.text ?? '')
            .filter(Boolean)
            .join('\n\n')
        : undefined)

  const params: Record<string, unknown> = {}
  for (const [k, v] of Object.entries(req)) {
    if (REQUEST_HANDLED.has(k)) continue
    params[k] = v
  }
  const responseFields: Record<string, unknown> = {}
  for (const [k, v] of Object.entries(res)) {
    if (RESPONSE_HANDLED.has(k)) continue
    responseFields[k] = v
  }

  const attrUsage: LlmUsage = {
    input: num(attrs['gen_ai.usage.input_tokens']),
    output: num(attrs['gen_ai.usage.output_tokens']),
    cacheRead: num(attrs['gen_ai.usage.cache_read_tokens']),
    cacheWrite: num(attrs['gen_ai.usage.cache_write_tokens']),
    costUsd: num(attrs['gen_ai.usage.cost_usd']),
  }
  const responseUsage = usageOf(res.usage)
  const usage: LlmUsage = {
    input: responseUsage?.input ?? attrUsage.input,
    output: responseUsage?.output ?? attrUsage.output,
    cacheRead: responseUsage?.cacheRead ?? attrUsage.cacheRead,
    cacheWrite: responseUsage?.cacheWrite ?? attrUsage.cacheWrite,
    costUsd: responseUsage?.costUsd ?? attrUsage.costUsd,
  }

  const responseMessage = isRecord(res.message) ? res.message : undefined
  const responseBlocks = blocksOf(res.content ?? responseMessage?.content)

  return {
    model:
      str(res.model) ??
      str(req.model) ??
      str(attrs['gen_ai.request.model']) ??
      str(attrs['gen_ai.response.model']),
    provider:
      str(res.provider) ?? str(req.provider) ?? str(attrs['gen_ai.system']),
    stopReason:
      str(res.stop_reason) ??
      str(res.finish_reason) ??
      str(attrs['gen_ai.response.finish_reason']),
    usage: Object.values(usage).some((n) => n !== undefined)
      ? usage
      : undefined,
    systemPrompt,
    messages,
    tools,
    params,
    responseFields,
    responseBlocks,
    requestTruncatedBytes: truncatedBytesOf(rawInput),
    responseTruncatedBytes: truncatedBytesOf(rawOutput),
    request,
    response,
  }
}

/** `1234567` → `1.23M`, `48210` → `48.2k`, `641` → `641`. */
export function formatTokenCount(n: number): string {
  if (n >= 1_000_000)
    return `${(n / 1_000_000).toFixed(2).replace(/\.?0+$/, '')}M`
  if (n >= 10_000) return `${(n / 1_000).toFixed(1).replace(/\.0$/, '')}k`
  return n.toLocaleString('en-US')
}

/** `0.1030595` → `$0.103`, `12.5` → `$12.50`. */
export function formatCost(usd: number): string {
  if (usd >= 1) return `$${usd.toFixed(2)}`
  if (usd >= 0.01) return `$${usd.toFixed(3)}`
  return `$${usd.toPrecision(2)}`
}
