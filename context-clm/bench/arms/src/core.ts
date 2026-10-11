// Bench-only comparison arms for context-clm (never shipped): `sum` = Codex-style summary at 75% of the budget,
// `base` = no management (one final turn over the limit, then stop). Both use the same ruler, calibration,
// caps and readout as the CLM arm, so every arm hits the same wall.
import { createHash } from 'node:crypto'

export type Block = { type: string; [k: string]: any }
export type Msg = { role: string; content: Block[]; [k: string]: any }
export type Arm = 'sum' | 'base'
export type Cfg = { arm: Arm; budget: number }
export type Stats = {
  steps: number
  peak_tc: number
  stops: number
  summaries: number
  summary_failures: number
  summary_cost_usd: number
}
export type State = {
  raw0: Msg | null
  covered: number
  hash: string
  proj: Msg[]
  ratio: number
  rawSent: number
  finalized: boolean
  stopped: boolean
  lastKey: string | null
  lastOut: Msg[] | null
  lastTc: number
  /** Calibrated size at the last summary attempt (sum arm), so a summary that stays big is not redone every step. */
  sumTc: number
  budget: number
  stats: Stats
}
export type Summarize = (transcript: string) => Promise<{ text: string; cost: number }>

export const RESERVE = 2048
export const OBS_MAX_CHARS = 10_000

// From OpenAI Codex 0.146.0 (Apache-2.0, Copyright OpenAI, https://github.com/openai/codex):
// the compaction prompt and the summary prefix, quoted verbatim for the summary baseline.
export const COMPACT_PROMPT = `You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.

Include:
- Current progress and key decisions made
- Important context, constraints, or user preferences
- What remains to be done (clear next steps)
- Any critical data, examples, or references needed to continue

Be concise, structured, and focused on helping the next LLM seamlessly continue the work.`
export const SUMMARY_PREFIX = `Another language model started to solve this problem and produced a summary of its thinking process. You also have access to the state of the tools that were used by that language model. Use this to build on the work that has already been done and avoid duplicating work. Here is the summary produced by the other language model, use the information in this summary to assist with your own analysis:`

export const STOP_TEXT =
  'context budget exhausted: the final turn ended with the context still over the limit, so the run was stopped'

export const finalText = (tokens: number, limit: number) =>
  `[context limit reached: ~${tokens}/${limit} tokens] This is your final turn. Finish now: put your answer or ` +
  'result where the task asks for it. The run ends after this turn.'

export const readout = (tokens: number, limit: number) =>
  `\n[context: ~${tokens}/${limit} tokens${tokens > limit ? ', over the limit' : ''}]`

export const newState = (): State => ({
  raw0: null,
  covered: 1,
  hash: '',
  proj: [],
  ratio: 1,
  rawSent: 0,
  finalized: false,
  stopped: false,
  lastKey: null,
  lastOut: null,
  lastTc: 0,
  sumTc: 0,
  budget: 0,
  stats: { steps: 0, peak_tc: 0, stops: 0, summaries: 0, summary_failures: 0, summary_cost_usd: 0 },
})

export const textMsg = (role: 'user' | 'assistant', text: string): Msg => ({
  role,
  content: [{ type: 'text', text }],
  timestamp: Date.now(),
})

export const resultText = (m: Msg) =>
  m.content
    .filter((b) => b.type === 'text')
    .map((b) => b.text)
    .join('')

const withText = (m: Msg, text: string): Msg => ({ ...m, content: [{ type: 'text', text }] })
const digest = (x: unknown) => createHash('sha256').update(JSON.stringify(x)).digest('hex')

function pieces(blocks: Block[] = [], out: string[] = []): string[] {
  for (const b of blocks) {
    if (b.type === 'text' || b.type === 'thinking') out.push(b.text ?? '')
    else if (b.type === 'function_call') out.push(String(b.function_id ?? ''), JSON.stringify(b.arguments ?? {}))
    else if (b.type === 'function_result') pieces(b.content, out)
  }
  return out
}

/** Raw ruler: UTF-8 bytes of the text pieces / 4 (the CLM arm's uncalibrated count). */
export function rawCount(system: string, msgs: Msg[]): number {
  let n = Buffer.byteLength(system)
  for (const m of msgs) for (const p of pieces(m.content)) n += Buffer.byteLength(p)
  return Math.floor((n + 1) / 4)
}

const TOO_LONG =
  'Output truncated: the command printed more than the observation limit. Narrow it (head, tail, grep -m, wc -l) ' +
  'or write it to a file and search that file.'

export function truncate(out: string, max: number): string {
  if (out.length <= max) return out
  const half = Math.min(5000, Math.floor(max / 2))
  return `${TOO_LONG}\n\n[first ${half} chars]\n${out.slice(0, half)}\n[${out.length - 2 * half} chars omitted]\n[last ${half} chars]\n${out.slice(-half)}`
}

/** shell::exec JSON result -> `<output>\n\n(exit_code=N)`; any other result text is truncated as is. */
export function execBody(text: string, max = OBS_MAX_CHARS): string {
  let j: any
  try {
    j = JSON.parse(text)
  } catch {
    return truncate(text, max)
  }
  if (!j || typeof j !== 'object' || !('exit_code' in j)) return truncate(text, max)
  const out = [String(j.stdout ?? ''), String(j.stderr ?? '')].filter(Boolean).join('\n').trimEnd()
  const rc = j.exit_code ?? (j.timed_out ? 124 : -1)
  return `${out ? truncate(out, max) : '(no output)'}\n\n(exit_code=${rc})`
}

/** Cut the middle of the newest result when it does not fit the room left under the limit. */
export function capNewest(text: string, room: number, limit: number): string {
  if (Math.floor((Buffer.byteLength(text) + 1) / 4) <= room) return text
  const half = Math.floor(Math.max((Math.max(room, 128) - 48) * 4, 512) / 2)
  if (text.length <= 2 * half) return text
  return `${text.slice(0, half)}\n\n[${text.length - 2 * half} chars cut to stay inside the ${limit}-token context limit; run a narrower command]\n\n${text.slice(-half)}`
}

export function renderMsg(m: Msg): string {
  const text = m.content
    .map((b) => {
      if (b.type === 'text') return b.text
      if (b.type === 'thinking') return b.text ? `(thinking) ${b.text}` : ''
      if (b.type === 'function_call') return `-> ${b.function_id} ${JSON.stringify(b.arguments ?? {})}`
      if (b.type === 'function_result') return renderMsg({ role: '', content: b.content ?? [] })
      return `[${b.type}]`
    })
    .filter(Boolean)
    .join('\n')
    .trim()
  return text || '(empty)'
}

export const renderWindow = (msgs: Msg[]) => msgs.map((m) => `[[${m.role}]]\n${renderMsg(m)}`).join('\n\n')

/** One assemble step. `key` = `${turn_id}:${step}`; the same key resends the cached request without a new step. */
export async function step(
  s: State,
  req: { messages: Msg[]; system: string },
  cfg: Cfg,
  key: string | null,
  summarize: Summarize,
): Promise<{ messages: Msg[]; stop: boolean; tc: number }> {
  if (key && key === s.lastKey && s.lastOut) return { messages: s.lastOut, stop: s.stopped, tc: s.lastTc }
  const raw = req.messages
  if (!raw.length) return { messages: raw, stop: false, tc: 0 }
  const limit = Math.max(cfg.budget - RESERVE, 0)
  s.budget = cfg.budget
  if (raw.length < s.covered || (s.hash && digest(raw.slice(0, s.covered)) !== s.hash))
    Object.assign(s, { raw0: null, covered: 1, hash: '', proj: [], finalized: false, sumTc: 0 })
  // A stop ended the previous turn: this step starts a new run (the CLM arm resets its controller the same way).
  if (s.stopped) s.finalized = false
  if (!s.raw0) s.raw0 = raw[0]
  const head = s.raw0
  const cal = (msgs: Msg[]) => Math.floor(rawCount(req.system, msgs) * s.ratio)
  const fresh = raw.slice(s.covered)

  // Calibrate against the provider's prompt count of the request that produced the newest assistant.
  const u = [...fresh].reverse().find((m) => m.role === 'assistant' && m.usage)?.usage
  if (u && s.rawSent > 0) {
    const r = ((u.input ?? 0) + (u.cache_read ?? 0) + (u.cache_write ?? 0)) / s.rawSent
    if (r >= 0.5 && r <= 3) s.ratio = r
  }

  const lastRes = [...fresh].reverse().find((m) => m.role === 'function_result')
  let readoutAt = -1
  for (const m of fresh) {
    if (m.role !== 'function_result') {
      s.proj.push(m)
      continue
    }
    // Calibrated limit in raw units, less the readout this result will carry (same room as the CLM arm).
    const room =
      Math.floor(limit / s.ratio) -
      (Math.floor(readout(limit, limit).length / 4) + 2) -
      rawCount(req.system, [head, ...s.proj])
    s.proj.push(withText(m, capNewest(execBody(resultText(m)), room, limit)))
    if (m === lastRes) readoutAt = s.proj.length - 1
  }
  s.covered = raw.length
  s.hash = digest(raw)

  if (cfg.arm === 'sum') {
    const lone = s.proj.length === 1 && resultText(s.proj[0]).startsWith(SUMMARY_PREFIX)
    const size = cal([head, ...s.proj])
    const grew = !s.sumTc || size >= s.sumTc + Math.round(0.05 * cfg.budget)
    if (s.proj.length && !lone && grew && size >= Math.round(0.75 * cfg.budget)) {
      s.sumTc = size
      const r = await summarize(renderWindow([head, ...s.proj]))
      s.stats.summary_cost_usd += r.cost
      if (r.text) {
        s.proj = [textMsg('user', `${SUMMARY_PREFIX}\n${r.text}`)]
        s.stats.summaries++
        // The next summary waits for the next crossing; only a failed attempt keeps the pre-summary size.
        s.sumTc = cal([head, ...s.proj])
        readoutAt = -1
      } else s.stats.summary_failures++
    }
  }

  if (readoutAt >= 0 && limit) {
    const m = s.proj[readoutAt]
    s.proj[readoutAt] = withText(m, resultText(m) + readout(cal([head, ...s.proj]), limit))
  }

  const tc = cal([head, ...s.proj])
  let stop = false
  if (limit && tc > limit) {
    if (s.finalized) stop = true
    else {
      s.finalized = true
      s.proj.push(textMsg('user', finalText(tc, limit)))
    }
  } else s.finalized = false
  s.stats.steps++
  s.stats.peak_tc = Math.max(s.stats.peak_tc, tc)
  if (stop) s.stats.stops++
  s.stopped = stop
  const out = [head, ...s.proj]
  s.rawSent = rawCount(req.system, out)
  s.lastKey = key
  s.lastOut = out
  s.lastTc = tc
  return { messages: out, stop, tc }
}
