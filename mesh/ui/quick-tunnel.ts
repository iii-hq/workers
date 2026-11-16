import type { JsonValue } from '@iii-dev/console-ui'

type JsonRecord = { [key: string]: JsonValue }
const isRecord = (value: JsonValue | undefined): value is JsonRecord => value !== null && typeof value === 'object' && !Array.isArray(value)

// Listen hosts that also answer on 127.0.0.1, where quick-tunnel forwards.
const LOOPBACK_REACHABLE = new Set(['', '0.0.0.0', '::', '127.0.0.1', 'localhost'])
// OpenTunnel names a target's route after its id.
const LABEL = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/
// quick-tunnel's built-in targets when its entry has none: writing a list replaces them.
const DEFAULT_TARGETS: JsonRecord = { webhooks: 'http://127.0.0.1:3112' }
// The first OpenTunnel start creates an identity, which outlasts quick-tunnel's 30 s default.
const START_MS = 120_000
// The hub holds 10-minute leases.
const LEASE_SECONDS = 600
// quick-tunnel's limit on targets.
const MAX_TARGETS = 32
// A ${VAR} template is the operator's to check; a number may be written as a string.
const below = (value: JsonValue | undefined, min: number) => !(typeof value === 'string' && value.includes('${')) && !(Number(value) >= min)
// A target's port as quick-tunnel compares them; null for a template or a value that is not a URL.
function portOf(value: JsonValue | undefined) {
  if (typeof value !== 'string' || value.includes('${')) return null
  try {
    const url = new URL(value)
    return Number(url.port || (url.protocol === 'https:' ? 443 : 80))
  } catch { return null }
}

// quick-tunnel registers its entry with this form id; Compose names the entry <namespace>-<container>.
const FORM = 'quick-tunnel'
type Listed = { id?: string; metadata?: { ui_form?: string } | null }
/** The id of quick-tunnel's configuration entry, or null when none is registered. Several (another
 * project's, a renamed container) prefer the one in this mesh's namespace. */
export function quickTunnelEntryId(entries: Listed[], meshId: string): string | null {
  const ids = entries.filter((e) => e.metadata?.ui_form === FORM && typeof e.id === 'string').map((e) => e.id as string).sort()
  const sibling = meshId.endsWith('mesh') ? `${meshId.slice(0, -'mesh'.length)}${FORM}` : FORM
  return ids.includes(sibling) ? sibling : ids[0] ?? null
}

export type QuickTunnelPlan = {
  ok: boolean
  // What a fix changes, in plain words.
  changes: string[]
  // The other targets, whose public URLs change with the provider.
  others: string[]
  // The whole entry to write; null when the page must not write it.
  next: JsonValue | null
  // Why the page cannot set it up; '' when it can.
  blocked: string
}

// What quick-tunnel's raw entry needs to carry the hub's tunnel `id` to the hub's `listen` address.
export function quickTunnelPlan(config: JsonValue | undefined, id: string, listen: string): QuickTunnelPlan {
  const no = (blocked: string): QuickTunnelPlan => ({ ok: false, changes: [], others: [], next: null, blocked })
  if (listen.includes('${')) return no(`The hub's listen address is ${listen}, so the page cannot tell its port. In quick-tunnel's settings, add the target ${id} as http://127.0.0.1:<port> and list it in proxy_protocol.`)
  const [, host = '', port = ''] = /^\[?(.*?)\]?:(\d+)$/.exec(listen) ?? []
  if (!port || !LOOPBACK_REACHABLE.has(host)) return no(`The tunnel forwards to 127.0.0.1, but the hub listens on ${listen}. Set the hub's Listen address to 0.0.0.0 or 127.0.0.1 in the mesh settings first.`)
  if (!(Number(port) >= 1 && Number(port) <= 65535)) return no(`The hub listens on port ${port}, which the tunnel cannot forward to. Set a port from 1 to 65535 in the hub's Listen address first.`)
  if (!LABEL.test(id)) return no(`The tunnel id ${id} cannot name an OpenTunnel target: use lowercase letters, digits and hyphens.`)
  const raw = no('quick-tunnel\'s settings hold a raw value the page does not edit. Change them in quick-tunnel\'s settings.')
  if (!isRecord(config)) return raw
  const targets = config.targets ?? DEFAULT_TARGETS
  const protocols = config.proxy_protocol ?? []
  if (!isRecord(targets) || !Array.isArray(protocols)) return raw
  const switching = config.provider !== 'opentunnel'
  const others = Object.keys(targets).filter((key) => key !== id)
  // OpenTunnel takes only http:// targets with lowercase names; a ${VAR} value is the operator's to check.
  const unfit = switching ? others.filter((key) => {
    const value = targets[key]
    return typeof value === 'string' && !value.includes('${') && (!LABEL.test(key) || !value.startsWith('http://'))
  }) : []
  if (unfit.length) return no(`OpenTunnel cannot carry quick-tunnel's ${unfit.join(', ')} target: it takes only http:// targets with lowercase names. Change it in quick-tunnel's settings first.`)
  // quick-tunnel refuses a port shared by targets in and out of proxy_protocol.
  const clash = others.filter((key) => !protocols.includes(key) && portOf(targets[key]) === Number(port))
  if (clash.length) return no(`quick-tunnel's ${clash.join(', ')} target also uses port ${port}, and quick-tunnel needs every target on a port to take each visitor's address or none. Move it to another port in quick-tunnel's settings first.`)
  if (!(id in targets) && others.length >= MAX_TARGETS) return no(`quick-tunnel already carries ${MAX_TARGETS} targets, its limit. Remove one in quick-tunnel's settings first.`)
  const origin = `http://127.0.0.1:${port}`
  // Only what must change is set: every other key and value, ${VAR} templates included, is written back as read.
  const next: JsonRecord = { ...config }
  const changes: string[] = []
  if (switching) {
    next.provider = 'opentunnel'
    changes.push(config.provider === undefined || config.provider === 'cloudflared' ? 'use OpenTunnel instead of cloudflared' : 'use OpenTunnel as the provider')
  }
  if (String(targets[id]).replace(/\/$/, '') !== origin) {
    next.targets = { ...targets, [id]: origin }
    changes.push(id in targets ? `point the target ${id} at ${origin}` : `add the target ${id}: ${origin}`)
  }
  if (!protocols.includes(id)) {
    next.proxy_protocol = [...protocols, id]
    changes.push(`pass each visitor's address to the hub (proxy_protocol: ${id})`)
  }
  if (below(config.startup_timeout_ms, START_MS)) {
    next.startup_timeout_ms = START_MS
    changes.push('allow 2 minutes for the tunnel to start')
  }
  if (config.max_lease_seconds !== undefined && below(config.max_lease_seconds, LEASE_SECONDS)) {
    next.max_lease_seconds = LEASE_SECONDS
    changes.push('let the hub keep the tunnel for 10 minutes at a time')
  }
  return { ok: changes.length === 0, changes, others: switching ? others : [], next, blocked: '' }
}
