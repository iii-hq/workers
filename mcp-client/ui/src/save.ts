import { errorMessage } from '@iii-dev/console-ui/format'
import { serverAccess } from './access'
import type { Api } from './api'
import { connectionChanged, REFERENCE } from './config-model'
import { looksSecretName } from './secret-names'
import type { Effects, Issue, Server, ServerValidation } from './types'

/** Save, approve, remove and turn on: the order of the calls is the security decision. The page never
 * computes a fingerprint; it approves the one `config::validate` returned for what the person read. */

export interface SaveDeps {
  api: Api
  confirm: (body: { title: string; lines: string[]; tone?: 'danger' }) => Promise<boolean>
  removeBlock: (server: string) => Promise<{ ok: boolean; reason?: string; count: number }>
  blockCount: (server: string) => Promise<number>
}

const nameOf = (ref: string) => ref.slice(ref.indexOf('://') + 3)
const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`
const blockSentence = (n: number) =>
  `The approval gate allows ${plural(n, 'function', 'functions')} under this name. Those rules will be removed; allow reads again once the server is connected.`
const gateError = (what: 'saved' | 'approved' | 'removed', verb: 'checked' | 'removed', reason: string) =>
  `Nothing was ${what}. The approval-gate rules could not be ${verb}: ${reason}`

/** What approving an entry lets mcp-client do, one plain sentence per effect. */
export function effectSentences(effects: Effects): string[] {
  const sends = effects.sends.map(({ ref, host }) => {
    const name = nameOf(ref)
    if (host === 'process') return `mcp-client will pass ${name} to the process`
    if (host === ref) return `mcp-client will connect to the address stored in ${name}`
    const to = REFERENCE.test(host) ? `the address stored in ${nameOf(host)}` : host
    return `mcp-client will send ${name} to ${to}`
  })
  return effects.runs ? [`mcp-client will run \`${effects.runs}\``, ...sends] : sends
}

/** Whether a save asks first: the fingerprint is new or changed, and the entry runs or sends something. */
export function needsConfirm(before: { fingerprint: string } | null, after: ServerValidation): boolean {
  const changed = before === null || before.fingerprint !== after.fingerprint
  return changed && (Boolean(after.effects.runs) || after.effects.sends.length > 0)
}

/** What a save compares against: the stored entry and the fingerprint of the server named `name`, never of the
 * one selected; nothing for an add, or for a name the config or the worker does not hold. */
export function saveBefore(
  adding: boolean,
  name: string,
  config: Record<string, unknown> | null,
  servers: Server[] | undefined,
): { entry: unknown; fingerprint: string } | null {
  const server = servers?.find((s) => s.name === name)
  return adding || !server || !config || !(name in config)
    ? null
    : { entry: config[name], fingerprint: server.fingerprint }
}

const validated = async (api: Api, name: string, entry: unknown) =>
  (await api.validate({ [name]: entry })).servers[name] as ServerValidation | undefined

/** Validate, ask, clear a stale gate block, approve the validated fingerprint, write: in that order. */
export async function saveEntry(
  deps: SaveDeps,
  name: string,
  entry: Record<string, unknown>,
  before: { entry: unknown; fingerprint: string } | null,
): Promise<{ saved: boolean; errors: Issue[]; message?: string }> {
  const v = await validated(deps.api, name, entry)
  if (!v?.ok) return { saved: false, errors: v?.errors ?? [] }
  // Nothing the fingerprint covers changed: an entry an outside write left unapproved stays so.
  if (before !== null && before.fingerprint === v.fingerprint) {
    await deps.api.writeEntry(name, entry)
    return { saved: true, errors: [] }
  }
  let count = 0
  if (before === null || connectionChanged(before.entry, entry)) {
    try {
      count = await deps.blockCount(name)
    } catch (err) {
      return { saved: false, errors: [], message: gateError('saved', 'checked', errorMessage(err)) }
    }
  }
  if (needsConfirm(before, v) || count > 0) {
    const lines = [...effectSentences(v.effects), ...(count > 0 ? [blockSentence(count)] : [])]
    if (!(await deps.confirm({ title: `Save ${name}?`, lines }))) return { saved: false, errors: [] }
  }
  if (count > 0) {
    const removed = await deps.removeBlock(name)
    if (!removed.ok) return { saved: false, errors: [], message: gateError('saved', 'removed', String(removed.reason)) }
  }
  await deps.api.approve(name, v.fingerprint)
  await deps.api.writeEntry(name, entry)
  return { saved: true, errors: [] }
}

/** Approve the stored entry of an `unapproved` server, always after a confirm that says what it allows. The
 * gate's block goes whenever there is one: there is no earlier entry to compare, and an entry changed outside
 * the page may point anywhere. */
export async function approveEntry(deps: SaveDeps, server: Server): Promise<boolean> {
  const { mcpServers } = await deps.api.readConfig()
  const v = await validated(deps.api, server.name, mcpServers[server.name])
  if (!v?.ok) return false
  const count = await deps.blockCount(server.name).catch((err) => {
    throw new Error(gateError('approved', 'checked', errorMessage(err)))
  })
  const effects = effectSentences(v.effects)
  const lines = [
    ...(effects.length > 0 ? effects : [`mcp-client will connect to ${server.target}`]),
    ...(count > 0 ? [blockSentence(count)] : []),
  ]
  if (!(await deps.confirm({ title: `Approve ${server.name}?`, lines }))) return false
  if (count > 0) {
    const removed = await deps.removeBlock(server.name)
    if (!removed.ok) throw new Error(gateError('approved', 'removed', String(removed.reason)))
  }
  await deps.api.approve(server.name, v.fingerprint)
  return true
}

/** Remove a server after a danger confirm: its gate block first, then the entry. */
export async function removeEntry(deps: SaveDeps, server: Server): Promise<boolean> {
  const count = await deps.blockCount(server.name).catch((err) => {
    throw new Error(gateError('removed', 'checked', errorMessage(err)))
  })
  const fns = serverAccess(server).length
  const lines = [
    `Its ${plural(fns, 'function goes', 'functions go')} away and its sign-in is cleared.`,
    ...(count > 0 ? [`Its ${plural(count, 'approval-gate rule is', 'approval-gate rules are')} removed too.`] : []),
  ]
  if (!(await deps.confirm({ title: `Remove ${server.name}?`, tone: 'danger', lines }))) return false
  if (count > 0) {
    const removed = await deps.removeBlock(server.name)
    if (!removed.ok) throw new Error(gateError('removed', 'removed', String(removed.reason)))
  }
  await deps.api.writeEntry(server.name, null)
  return true
}

/** Edit as JSON's save: the whole map validated, then each removed or changed name, in name order, through
 * `removeEntry` or `saveEntry`. The first one declined or failed stops the rest. */
export async function saveMap(
  deps: SaveDeps,
  shown: Record<string, unknown>,
  edit: { candidate: Record<string, unknown>; changed: string[]; removed: string[] },
  listed: Server[],
): Promise<{ saved: boolean; issues: Issue[]; message?: string }> {
  const { servers } = await deps.api.validate(edit.candidate)
  const errors = Object.values(servers).flatMap((v) => v.errors)
  if (errors.length > 0) return { saved: false, issues: errors }
  const notes: Issue[] = []
  for (const name of [...edit.removed, ...edit.changed].sort()) {
    const server = listed.find((s) => s.name === name)
    if (edit.removed.includes(name)) {
      if (!server) notes.push({ path: '', message: `${name}: not removed, the worker does not list it` })
      else if (!(await removeEntry(deps, server))) return { saved: false, issues: notes }
      continue
    }
    const entry = edit.candidate[name] as Record<string, unknown>
    const result = await saveEntry(deps, name, entry, saveBefore(false, name, shown, listed))
    if (!result.saved) return { saved: false, issues: [...notes, ...result.errors], message: result.message }
  }
  return { saved: true, issues: notes }
}

/** Share an entry's references with mcp-client, after one confirm that names them (names only, never a value).
 * Every reference is read before any is written, so a missing one leaves every grant as it was. */
export async function shareEntry(deps: SaveDeps, name: string, refs: string[]): Promise<boolean> {
  if (!(await deps.confirm({ title: 'Share with mcp-client?', lines: refs }))) return false
  const consumers: string[][] = []
  for (const ref of refs) consumers.push(await deps.api.secretConsumers(ref))
  for (const [i, ref] of refs.entries()) await deps.api.grantSecret(ref, consumers[i])
  await deps.api.reconnect(name)
  return true
}

/** Turn a server off or on. `disabled` is outside the fingerprint: no validate, no approve. */
export async function setDisabled(api: Api, name: string, disabled: boolean): Promise<void> {
  const entry = (await api.readConfig()).mcpServers[name]
  if (typeof entry !== 'object' || entry === null || Array.isArray(entry)) throw new Error(`${name} is not configured`)
  const { disabled: _, ...rest } = entry as Record<string, unknown>
  await api.writeEntry(name, disabled ? { ...rest, disabled: true } : rest)
}

const decode = (segment: string) => segment.replace(/~1/g, '/').replace(/~0/g, '~')

/** The errors at a secret position: `auth.token`, `auth.client_secret`, or a secret-named header or variable. */
export function secretIssues(errors: Issue[]): Issue[] {
  return errors.filter(({ path }) => {
    const [field, name] = path.split('/').slice(-2).map(decode)
    if (field === 'auth') return name === 'token' || name === 'client_secret'
    return (field === 'headers' || field === 'env') && looksSecretName(name)
  })
}
