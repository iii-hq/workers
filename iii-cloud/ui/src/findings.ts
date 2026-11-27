// What the check found, as CheckStage's sections show it: one deduplicated list (every count comes from it),
// split by owner, then grouped by code and file ("21 variables have no value in iii cloud").

import { checklist, hiddenPerson, isDatabaseVar, problemDetail, problemTitle, VALUE_CODES, variableName } from './model'
import type { Problem, SecretRequest, Status, Suggestion, ValueItem, Values } from './types'

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
  /** Values the deploy cannot go without (required, not set): the one "N values need you" row, never a row each. */
  values: Item[]
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
  if (!c) return { person: [], agent: [], values: [], notes: [], skipped: [] }
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
  // A variable or a secret is not a row of the check: only the required ones that are missing, as one count.
  const isValue = (i: Item) => (!!i.code && VALUE_CODES.has(i.code)) || !!i.secret
  const shown = [...kept, ...warnings.filter((i) => !(i.code && codes.has(i.code)))]
  // They come from the check's own problems (the hand-off's checklist leaves them out), and from what the agent asked for.
  const values = [
    ...c.problems
      .filter(
        (p) =>
          VALUE_CODES.has(p.code) &&
          p.severity === 'block' &&
          p.owner === 'person' &&
          !hiddenPerson(s, p) &&
          !skippedHere(s, p),
      )
      .map((p) => ({ ...itemOf(p.id, p, { ...p, title: problemTitle(p) }, p.owner, false), secret: variableName(p) })),
    ...shown.filter((i) => !i.code && i.secret && !i.fixed),
  ]
  return {
    person: shown.filter((i) => !isValue(i)),
    agent: all.filter((i) => i.owner === 'agent'),
    values,
    notes: all.filter((i) => i.owner === 'none' && !isValue(i)),
    skipped: [...aside, ...byCode].filter((i) => !isValue(i)),
  }
}

/** The run here lacks this name and the person skipped it: it runs with an empty value; the old problem waits for the next check. */
const skippedHere = (s: Status, p: Problem) =>
  p.code === 'LOCAL_VARIABLE_UNSET' && (s.skipped_values ?? []).includes(variableName(p) ?? '')

/** O2: the workers' config, theirs to copy; a warning. */
export const configAction = (i: Item) => !!i.code && CONFIG_CODES.has(i.code)

/**
 * The sources the worker preselects for a name iii cloud has no value for: none for a name with a default, a set,
 * skipped, database or invalid one (a copy is never preselected). The one rule behind the Secrets screen's "can come from your
 * env files", the values card's count and the review dialog's own choices, so the three cannot disagree.
 */
export const preselected = (i: ValueItem): Suggestion[] =>
  i.need === 'has_default' || i.in_cloud !== 'unset' || i.skipped || i.database || i.invalid_name
    ? []
    : i.suggestions.filter((x) => x.preselect)

/** The names the compose file reads that iii cloud does not fill in by itself, told apart for the values card. */
export function valuesPlan(v: Values | null) {
  // The database is iii cloud's, unless the person chose one outside it and the agent asks for its URL.
  const items = (v?.items ?? []).filter((i) => !i.database || i.asked)
  // An agent's ask is the agent's reason first: the worker marks it `required` too, though the file may not read it.
  // `unknown`: iii cloud's variables could not be read, so nothing is known to be missing.
  const unset = items.filter((i) => i.in_cloud === 'unset' && i.need !== 'has_default')
  const asked = unset.filter((i) => i.asked)
  const rest = unset.filter((i) => !i.asked)
  // Set in iii cloud, but the run here has no value (`LOCAL_VARIABLE_UNSET`, which blocks the check): an env file of
  // the folder gives it, or a skip runs it with an empty value.
  const local = items.filter((i) => i.in_cloud === 'set' && i.here === 'unset' && !i.skipped)
  return {
    items,
    /** Required by the file and not set: what stops a deploy. */
    required: rest.filter((i) => i.need === 'required'),
    /** Asked for by the agent (a secret request). */
    asked,
    /** Set in iii cloud and missing on this machine. */
    local,
    optional: rest.filter((i) => i.need !== 'required' && !i.skipped),
    skipped: [
      ...rest.filter((i) => i.need !== 'required' && i.skipped),
      ...items.filter((i) => i.in_cloud === 'set' && i.skipped),
    ],
    set: items.filter((i) => i.in_cloud === 'set' && !local.includes(i) && !i.skipped),
    /** Not set, and an env file or this environment's address can give the value: the Secrets screen's "can come from your env files". */
    offered: items.filter((i) => preselected(i).length > 0),
  }
}

/**
 * What only the person can do before this check can pass: its blocking problems (a warning is not one) and the
 * console settings still to decide. The one list the Deploy screen's title and the environment's next action count.
 * `read`: the worker's values read. The values the deploy cannot go without are ONE thing, the values card; they are
 * never rows of their own (`values` names them, for the sentence that says how many).
 */
export function needsYou(s: Status, read: Values | null = null) {
  const plan = valuesPlan(read)
  // A name the read says is set (and the run has) is not missing, whatever the check's problems or the agent's waiting
  // request still say.
  const fine = new Set(plan.set.map((i) => i.name))
  const names = new Set(
    [
      ...plan.required.map((i) => i.name),
      ...plan.local.map((i) => i.name),
      ...plan.asked.map((i) => i.name),
      ...findings(s).values.map((i) => valueName(i) ?? i.key),
    ].filter((n) => !fine.has(n)),
  )
  const items = findings(s).person.filter((i) => !i.fixed && i.severity === 'block' && !configAction(i))
  const chosen = s.override_choices ?? {}
  const decide = (s.check?.plan.confirmations.overrides ?? []).filter((o) => !chosen[o.setting])
  // The agent's question is one more thing for the person, counted here for every screen. Not twice: a question that
  // names a value already counted (the same name asked through `iii-cloud::secret` and in the chat) is that value.
  const q = s.agent?.state === 'waiting_for_you' ? (s.agent.question ?? '') : null
  const asks = q !== null && ![...names].some((n) => q.includes(n))
  return {
    items,
    decide,
    values: [...names],
    asks,
    count: items.length + decide.length + (names.size ? 1 : 0) + (asks ? 1 : 0),
  }
}
