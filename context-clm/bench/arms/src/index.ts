// context-arms: bench-only worker registering context-sum::assemble and context-base::assemble (+ ::status).
// It needs III_URL on purpose: a bare start would register on whatever engine the SDK defaults to (the live stack).
import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { registerWorker } from 'iii-sdk'
import {
  type Arm,
  COMPACT_PROMPT,
  newState,
  RESERVE,
  renderWindow,
  STOP_TEXT,
  type State,
  step,
  textMsg,
} from './core.ts'

const url = process.env.III_URL
if (!url) throw new Error('set III_URL to the bench engine, e.g. ws://127.0.0.1:49250')
const DIR = process.env.ARMS_DIR ?? '/tmp/context-arms'
const DEFAULT_BUDGET = Number(process.env.ARMS_BUDGET_TOKENS ?? 65536)
mkdirSync(join(DIR, 'window'), { recursive: true, mode: 0o700 })

const iii = registerWorker(url, { workerName: 'context-arms' })
// NOTE: in-memory state, one process; a restart mid-episode rebuilds from the raw window (bench-only worker).
// One map per arm: a status call must never see another arm's state for the same session id.
const sessions: Record<Arm, Map<string, State>> = { sum: new Map(), base: new Map() }
const safe = (sid: string) => sid.replace(/[^\w.-]/g, '_')

const summarizer = (sid: string, model: any, system: string) => async (transcript: string) => {
  try {
    const r: any = await iii.trigger({
      function_id: 'router::complete',
      payload: {
        session_id: sid,
        model: model.id,
        provider: model.provider,
        system_prompt: system,
        tools: [],
        max_output_tokens: 16000,
        messages: [textMsg('user', transcript), textMsg('user', COMPACT_PROMPT)],
      },
      timeoutMs: 300_000,
    })
    const text = (r?.message?.content ?? [])
      .filter((b: any) => b.type === 'text')
      .map((b: any) => b.text)
      .join('')
      .trim()
    const cost = Number(r?.usage?.cost_usd ?? r?.message?.usage?.cost_usd ?? 0)
    return { text: r?.message?.stop_reason === 'end' ? text : '', cost }
  } catch {
    return { text: '', cost: 0 }
  }
}

for (const arm of ['sum', 'base'] as Arm[]) {
  iii.registerFunction(
    `context-${arm}::assemble`,
    async (req: any) => {
      const o = req.options ?? {}
      const sid: string = o.session_id ?? o.lease_key ?? ''
      const s = sessions[arm].get(sid) ?? newState()
      sessions[arm].set(sid, s)
      const budget = Number(o.budget_tokens ?? DEFAULT_BUDGET)
      const system = req.system_prompt ?? ''
      const key = o.turn_id != null && o.step != null ? `${o.turn_id}:${o.step}` : null
      const r = await step(
        s,
        { messages: req.messages, system },
        { arm, budget },
        key,
        summarizer(sid, req.model, system),
      )
      writeFileSync(join(DIR, 'window', `${safe(sid)}.txt`), renderWindow(r.messages), { mode: 0o600 })
      const res: any = await iii.trigger({
        function_id: 'context::assemble',
        payload: { ...req, messages: r.messages, options: { ...o, allow_compaction: false, allow_prune: false } },
        timeoutMs: 300_000,
      })
      if (!r.stop) return res
      const limit = budget - RESERVE
      return { ...res, usable: limit, token_count: limit + 1, applied: { ...res.applied, strategy_stop: STOP_TEXT } }
    },
    {
      description: `context::assemble with the bench '${arm}' arm (context-clm comparison)`,
      metadata: { internal: true },
    },
  )
  iii.registerFunction(
    `context-${arm}::status`,
    async (req: any) => {
      const s = sessions[arm].get(req?.session_id ?? '')
      return s ? { found: true, budget_tokens: s.budget, stopped: s.stopped, stats: s.stats } : { found: false }
    },
    { description: `per-session counters of the bench '${arm}' arm`, metadata: { internal: true } },
  )
}
console.log(`context-arms ready: dir=${DIR} default_budget=${DEFAULT_BUDGET}`)
