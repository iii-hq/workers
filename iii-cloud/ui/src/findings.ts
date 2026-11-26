// What the check found, as CheckStage's sections show it: one deduplicated list (every count comes from it),
// split by owner, then grouped by code and file ("21 variables have no value in iii cloud").

import { checklist, isDatabaseVar, problemDetail, problemTitle, variableName } from './model'
import type { Problem, SecretRequest, Status } from './types'

export type Item = {
  key: string
  /** The current check's problem; null for a row the agent fixed (gone from the check). */
  problem: Problem | null
  title: string
  code: string | null
  file: string | null
  line: number | null
  /** The YAML path the cloud names (`containers.http.worker`). */
  target: string | null
  /** What it is about, for a group's chips: the variable, the container, the setting. */
  name: string | null
  /** The cloud's words without "See file:line." and "(at …)": those make the location line. */
  detail: string | null
  /** What to do about it, in the check's words (`Problem.fix`). */
  fix: string | null
  owner: Problem['owner']
  /** A fixed row has no severity (the hand-off's brief does not keep it): it counts as blocking. */
  severity: Problem['severity']
  fixed: boolean
  /** The value the person gives ("Set values"): the variable a problem names, or one the agent asked for. */
  secret: string | null
  request?: SecretRequest
  /** The person skipped its name for now (`values::skip`). */
  skipped?: boolean
}

/** O2: the workers' config, which the person copies to iii cloud (or skips, or sets in the console). */
const CONFIG_CODES = new Set(['LOCAL_CONFIG_NOT_SET'])

const AT = /\s*\(at ([^()]+)\)\s*$/
const SEE = /^See \S+?\.(?=\s|$)\s*/

/** `See api/x.yaml:81. (at containers.api.environment.KEY)` → no text, target `containers.api.environment.KEY`. */
export function splitDetail(detail: string | null): { text: string | null; target: string | null } {
  if (!detail) return { text: null, target: null }
  const at = detail.match(AT)
  const text = (at ? detail.slice(0, at.index) : detail).replace(SEE, '').trim()
  return { text: text || null, target: at ? at[1].trim() : null }
}

/** The name a target is about: a container's variable, else the container, else the last key. */
export function targetName(target: string | null): string | null {
  if (!target) return null
  const seg = target.split('.')
  if (seg[0] === 'containers' && seg.length > 1) {
    const env = seg.indexOf('environment')
    return env > 0 && seg[env + 1] ? seg.slice(env + 1).join('.') : seg[1]
  }
  return seg.at(-1) || null
}

/** The key on the problem's own line of the excerpt (`  OPENAI_API_KEY: ${OPENAI_API_KEY:-}` → OPENAI_API_KEY). */
function excerptKey(p: Problem | null): string | null {
  const x = p?.excerpt
  if (!x || p?.line == null) return null
  return x.lines[p.line - x.start_line]?.match(/^\s*-?\s*([A-Za-z_][\w.-]*)\s*:/)?.[1] ?? null
}

/** One faint line: `api/worker-compose-prod.yaml:81 · containers.api.environment.KEY`. */
export function location(file: string | null, line: number | null, target: string | null): string | null {
  const at = file ? (line != null ? `${file}:${line}` : file) : null
  return [at, target].filter(Boolean).join(' · ') || null
}

const RANK: Record<Problem['severity'], number> = { block: 0, warn: 1, info: 2 }

/** The same file, line and target twice (two codes for one line): keep the open one, then the most severe. */
function dedupe(items: Item[]): Item[] {
  const seen = new Map<string, number>()
  const out: Item[] = []
  for (const it of items) {
    const key = it.file && it.line != null ? `${it.file}:${it.line}|${it.target ?? ''}` : null
    const at = key ? seen.get(key) : undefined
    if (key && at === undefined) seen.set(key, out.length)
    if (at === undefined) {
      out.push(it)
      continue
    }
    const kept = out[at]
    const better = (kept.fixed && !it.fixed) || (kept.fixed === it.fixed && RANK[it.severity] < RANK[kept.severity])
    if (better) out[at] = it
  }
  return out
}

function itemOf(
  key: string,
  p: Problem | null,
  row: { title: string; code: string | null; file: string | null; line: number | null; detail: string | null },
  owner: Problem['owner'],
  fixed: boolean,
): Item {
  const { text, target } = splitDetail(p ? problemDetail(p) : row.detail)
  return {
    key,
    problem: p,
    title: row.title,
    code: p?.code ?? row.code,
    file: row.file,
    line: row.line,
    target,
    name: targetName(target) ?? excerptKey(p) ?? (p ? variableName(p) : null),
    detail: text,
    fix: p?.fix ?? null,
    owner,
    severity: p?.severity ?? 'block',
    fixed,
    secret: null,
  }
}

export type Findings = {
  /** "You": the person's open problems, the ones they did, and the values the agent asked for. */
  person: Item[]
  /** "Your agent": the agent's, open and fixed. */
  agent: Item[]
  /** Information only: nobody acts on it, never counted. */
  notes: Item[]
  /** "Skipped": one row per name skipped for now, never counted (a skipped SECRET_MISSING stays in `person`). */
  skipped: Item[]
}

/** What a skipped SECRET_MISSING says: skipping it is not enough for iii cloud. */
export const SKIPPED_SECRET = 'iii cloud needs a value to deploy, even an empty one.'
/** The variable a value row is about. */
export const valueName = (i: Item) => i.secret ?? i.name ?? variableName(i)

/** The one list: the hand-off's checklist (fixed rows kept) and the notes, deduplicated, by owner. */
export function findings(s: Status): Findings {
  const c = s.check
  if (!c) return { person: [], agent: [], notes: [], skipped: [] }
  const byId = new Map(c.problems.map((p) => [p.id, p]))
  const rows = checklist(s).rows.map((r) => {
    const p = r.pid ? (byId.get(r.pid) ?? null) : null
    return itemOf(r.id, p, r, r.owner as Problem['owner'], r.fixed)
  })
  const notes = c.problems
    .filter((p) => p.owner === 'none')
    .map((p) => itemOf(p.id, p, { ...p, title: problemTitle(p) }, 'none', false))
  const all = dedupe([...rows, ...notes])
  const env = c.environment_id ?? s.project?.environment?.id
  // iii cloud provides the database: its URL is asked for only when the person keeps one outside it.
  const waiting = s.secrets.filter(
    (r) => r.state === 'waiting' && r.environment_id === env && (r.external || !isDatabaseVar(r.name)),
  )
  const person = all
    .filter((i) => i.owner === 'person')
    .map((i) => {
      if (i.fixed || i.problem?.code !== 'LOCAL_VARIABLE_UNSET') return i
      const secret = variableName(i.problem)
      return { ...i, secret, request: secret ? waiting.find((r) => r.name === secret) : undefined }
    })
  for (const r of waiting) {
    if (person.some((i) => i.secret === r.name)) continue
    person.push({
      key: r.id,
      problem: null,
      title: `Your agent asked for ${r.name}`,
      code: null,
      file: null,
      line: null,
      target: null,
      name: r.name,
      detail: r.description,
      fix: null,
      owner: 'person',
      severity: 'block',
      fixed: false,
      secret: r.name,
      request: r,
    })
  }
  // A skipped name leaves the person's list (its row gone from the check would read "fixed"), except a
  // SECRET_MISSING still in the check: iii cloud needs a value for it, even an empty one.
  const skipped = new Set([
    ...(s.skipped_values ?? []),
    ...person.flatMap((i) => (i.problem?.skipped ? [valueName(i) ?? ''] : [])).filter(Boolean),
  ])
  const mine = person.flatMap((i) => {
    const name = valueName(i)
    if (!i.problem?.skipped && !(name && skipped.has(name))) return [i]
    if (i.fixed || i.code !== 'SECRET_MISSING') return []
    return [{ ...i, skipped: true, secret: name, detail: SKIPPED_SECRET }]
  })
  const stays = new Set(mine.filter((i) => i.skipped).map(valueName))
  // O2: a warning skipped by code (`problem::skip`): the status says which now; the check marks it (as info) from
  // its next run on, when it leaves the open problems and the hand-off's row for it would read "fixed".
  const codes = new Set(
    s.skipped_problems ?? c.problems.filter((p) => p.skipped && CONFIG_CODES.has(p.code)).map((p) => p.code),
  )
  const marked = c.ready ? [] : c.problems.filter((p) => p.skipped && p.owner === 'person' && CONFIG_CODES.has(p.code))
  const gone = new Set(marked.map(problemTitle))
  const warnings = [
    ...mine.filter((i) => !i.fixed && i.code && CONFIG_CODES.has(i.code)),
    ...marked.map((p) => itemOf(p.id, p, { ...p, title: problemTitle(p) }, 'person', false)),
  ]
  const kept = mine.filter((i) => !(i.code && CONFIG_CODES.has(i.code)) && !(i.fixed && gone.has(i.title)))
  const byCode = warnings
    .filter((i) => i.code && codes.has(i.code))
    .map((i) => ({ ...i, skipped: true, severity: 'info' as const }))
  const aside: Item[] = [...skipped]
    .filter((name) => !stays.has(name))
    .map((name) => ({
      key: `skipped:${name}`,
      problem: null,
      title: `${name} has no value`,
      code: 'SKIPPED_VALUE',
      file: null,
      line: null,
      target: null,
      name,
      detail: null,
      fix: null,
      owner: 'person',
      severity: 'info',
      fixed: false,
      secret: name,
      skipped: true,
    }))
  return {
    person: [...kept, ...warnings.filter((i) => !(i.code && codes.has(i.code)))],
    agent: all.filter((i) => i.owner === 'agent'),
    notes: all.filter((i) => i.owner === 'none'),
    skipped: [...aside, ...byCode],
  }
}

/** O2: the workers' config, theirs to copy; a warning. */
export const configAction = (i: Item) => !!i.code && CONFIG_CODES.has(i.code)

/**
 * What only the person can do before this check can pass: its blocking problems (a warning is not one) and the
 * console settings still to decide. The one list the Deploy screen's title and the environment's next action count.
 * `except`: names a bulk row covers instead.
 */
export function needsYou(s: Status, except: Set<string> = new Set()) {
  const items = findings(s).person.filter(
    (i) => !i.fixed && i.severity === 'block' && !configAction(i) && !except.has(valueName(i) ?? ''),
  )
  const chosen = s.override_choices ?? {}
  const decide = (s.check?.plan.confirmations.overrides ?? []).filter((o) => !chosen[o.setting])
  return { items, decide }
}
