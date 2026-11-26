// What the check found, as CheckStage's sections show it: one deduplicated list (every count comes from it),
// split by owner, then grouped by code and file ("21 variables have no value in iii cloud").

import { checklist, groupDetail, groupTitle, isDatabaseVar, problemDetail, problemTitle, variableName } from './model'
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

export type Group = {
  key: string
  code: string | null
  file: string | null
  items: Item[]
}

/** Values the person gives with "Set values". */
export const VALUE_CODES = new Set(['EMPTY_DEFAULT_VARIABLE', 'LOCAL_VARIABLE_UNSET', 'SECRET_MISSING'])
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

/** The excerpt's code: nothing when its lines are blank or only repeat where it is. */
export function codeLines(p: Problem | null): string[] {
  const x = p?.excerpt
  if (!x) return []
  const where = p?.file ? [p.file, p.line != null ? `${p.file}:${p.line}` : p.file] : []
  return x.lines.some((l) => l.trim() && !where.includes(l.trim())) ? x.lines : []
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
    owner,
    severity: p?.severity ?? 'block',
    fixed,
    secret: null,
  }
}

export type Findings = {
  /** "Only you can do this": the person's open problems, the ones they did, and the values the agent asked for. */
  person: Item[]
  /** "What the check found": the agent's, open and fixed. */
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

/** A value the person gives with "Set values": theirs to do, not a note. */
export const personAction = (i: Item) => !!i.secret || (!!i.code && VALUE_CODES.has(i.code))
/** O2: the workers' config, theirs to copy; a warning, so "Worth a look" leads with it. */
export const configAction = (i: Item) => !!i.code && CONFIG_CODES.has(i.code)

/**
 * Within a section, the problems sharing a code and a file form one group (first seen first). A fixed row has
 * no code: it joins the open rows of its title.
 */
export function groupItems(items: Item[]): Group[] {
  const codeOf = new Map<string, string>()
  for (const it of items) if (it.code) codeOf.set(it.title, it.code)
  const groups = new Map<string, Group>()
  for (const it of items) {
    const code = it.code ?? codeOf.get(it.title) ?? null
    const key = `${code ?? `title:${it.title}`}|${it.file ?? ''}${it.skipped ? '|skipped' : ''}`
    const g = groups.get(key)
    if (g) g.items.push(it)
    else groups.set(key, { key, code, file: it.file, items: [it] })
  }
  return [...groups.values()]
}

/** The group's sentence, count first; one item keeps its own title. */
export const titleOf = (g: Group) =>
  g.items.length > 1 ? groupTitle(g.code, g.items[0].title, g.items.length) : g.items[0].title

/** A group's location: its file, its lines (`81–101` when they follow each other, else `40, 46, 52`; mixed `81–94, 96`), and the path its targets share (`…environment`). */
export function groupLocation(g: Group): string | null {
  const lines = [...new Set(g.items.map((i) => i.line).filter((l): l is number => l != null))].sort((a, b) => a - b)
  // Runs of following lines read as spans: 81–94, 96, 98–101.
  const runs: number[][] = []
  for (const l of lines) {
    const last = runs.at(-1)
    if (last && l === last[last.length - 1] + 1) last.push(l)
    else runs.push([l])
  }
  const span = runs.map((r) => (r.length > 2 ? `${r[0]}–${r.at(-1)}` : r.join(', '))).join(', ')
  const at = g.file ? (lines.length ? `${g.file}:${span}` : g.file) : null
  const parents = new Set(g.items.map((i) => i.target?.split('.').slice(0, -1).join('.') ?? ''))
  const [parent] = parents
  return [at, parents.size === 1 && parent ? parent : null].filter(Boolean).join(' · ') || null
}

/** The group's own sentence ("Each one defaults to empty …"), else the detail every item shares, said once. */
export function sharedDetail(g: Group): string | null {
  const first = g.items[0].detail
  return groupDetail(g.code) ?? (first && g.items.every((i) => i.detail === first) ? first : null)
}
