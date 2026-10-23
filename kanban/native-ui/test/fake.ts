/**
 * A fake kanban worker behind the test host: columns, tickets with comments
 * and activity, agents, projects and releases that the calls read and
 * change the way kanban/src/board.rs does (a move renumbers its column and
 * compacts the one it left; a create or a status change goes to the end),
 * the `kanban:change` events those writes fire, and a way to hold a call
 * open or to answer it otherwise.
 */

import { createTestHost, type Env, HostError, type TestHost, type TestHostOptions } from '@iii-dev/native-ui/testing'
import { inject } from 'vitest'

type Payload = Record<string, any>

export const HOUR = 3_600_000

/** The host holds a bound enum field as its index (catalog.json › enums). */
export const HUE = { none: 0, blue: 1, purple: 2, teal: 3, green: 4, amber: 5, rose: 6 }
export const VARIANT = { default: 0, ok: 1, warn: 2, alert: 3, accent: 4 }
/** Timestamps are real time (the VM's `Date.now` is): `T(-2 * HOUR)` is two hours ago. */
export const T = (offset = 0) => new Date(Date.now() + offset).toISOString()

export interface FakeTicket {
  id: string
  key: string
  title: string
  description: string
  status: string
  priority: string
  assignee: string | null
  labels: string[]
  pills: { icon: string; text: string }[]
  order: number
  created_at: string
  updated_at: string
  deleted_at: string | null
  created_by: string
  parent_id: string | null
  project_id: string | null
  release_id: string | null
  comments: Payload[]
  activity: Payload[]
}

export interface Held {
  resolve(value: unknown): void
  reject(err: Error): void
  payload: Payload
}

export const COLUMNS = [
  { id: 'backlog', label: 'Backlog' },
  { id: 'todo', label: 'To do' },
  { id: 'in_progress', label: 'In progress' },
  { id: 'in_review', label: 'In review' },
  { id: 'done', label: 'Done' },
  { id: 'canceled', label: 'Canceled' },
]

export const AGENTS = [
  { id: 'tech-lead', name: 'Tech Lead', color: 'purple' },
  { id: 'backend-engineer', name: 'Backend Engineer', color: 'teal' },
  { id: 'frontend-engineer', name: 'Frontend Engineer', color: 'not-a-hue' },
]

export class FakeKanban {
  columns = COLUMNS.map((c) => ({ ...c }))
  priorities = ['low', 'medium', 'high', 'urgent']
  prefix = 'KAN'
  tickets: FakeTicket[] = []
  agents: Payload[] = AGENTS.map((a) => ({ ...a }))
  projects: Payload[] = [
    { id: 'p1', name: 'iii cloud', color: 'blue', lead: 'tech-lead', archived_at: null },
    { id: 'p2', name: 'Old site', color: 'rose', lead: null, archived_at: T(-100 * HOUR) },
  ]
  releases: Payload[] = [
    { id: 'r1', name: 'Phase 2', target_at: '2026-10-30', released_at: null },
    { id: 'r0', name: 'Phase 1', target_at: '2026-09-01', released_at: T(-200 * HOUR) },
  ]
  next = 1
  seq = 1
  /** The host to send `kanban:change` to, when set (`emit`). */
  host: TestHost | null = null
  /** Answers by function id (without `kanban::`) that win over the fake's own. */
  overrides: Record<string, (payload: Payload) => unknown> = {}
  private held = new Map<string, Held[]>()
  private holding = new Set<string>()
  private clock = Date.now() - 1000 * HOUR

  /** A write's timestamp: strictly increasing, like `next_at`. */
  stamp(): string {
    this.clock = Math.max(this.clock + 1000, Date.now() - 1000)
    return new Date(this.clock).toISOString()
  }

  add(fields: Partial<FakeTicket> & { title: string }): FakeTicket {
    const n = this.next++
    const status = fields.status ?? 'backlog'
    const at = fields.updated_at ?? this.stamp()
    const t: FakeTicket = {
      id: `t${n}`,
      key: `${this.prefix}-${n}`,
      description: '',
      status,
      priority: 'medium',
      assignee: null,
      labels: [],
      pills: [],
      order: this.max(status) + 1,
      created_at: at,
      updated_at: at,
      deleted_at: null,
      created_by: 'user',
      parent_id: null,
      project_id: null,
      release_id: null,
      comments: [],
      activity: [
        {
          id: `a${this.seq}`,
          ticket_id: `t${n}`,
          seq: this.seq++,
          type: 'ticket.created',
          actor: 'user',
          at,
          changes: [{ field: 'status', from: null, to: status }],
        },
      ],
      ...fields,
    }
    this.tickets.push(t)
    return t
  }

  get(ref: string): FakeTicket | undefined {
    const r = ref.toLowerCase()
    return this.tickets.find((t) => t.id === ref || t.key.toLowerCase() === r)
  }

  private max(status: string): number {
    return this.tickets.filter((t) => t.status === status && !t.deleted_at).reduce((m, t) => Math.max(m, t.order), 0)
  }

  /** A column as the worker orders it. */
  column(status: string): FakeTicket[] {
    return this.tickets
      .filter((t) => t.status === status && !t.deleted_at)
      .sort((a, b) => a.order - b.order || (a.created_at < b.created_at ? -1 : 1))
  }

  summary(t: FakeTicket): Payload {
    const { description: _d, comments, activity: _a, created_by: _c, ...rest } = t
    return { ...rest, comment_count: comments.length }
  }

  view(t: FakeTicket): Payload {
    const children = this.tickets.filter((c) => c.parent_id === t.id && !c.deleted_at)
    const parent = t.parent_id ? this.tickets.find((p) => p.id === t.parent_id) : undefined
    const brief = (b: FakeTicket) => ({
      id: b.id,
      key: b.key,
      title: b.title,
      status: b.status,
      priority: b.priority,
      assignee: b.assignee,
      deleted_at: b.deleted_at,
      project_id: b.project_id,
      release_id: b.release_id,
    })
    return {
      ...t,
      parent: parent ? brief(parent) : null,
      children: children.map(brief),
      progress: {
        done: children.filter((c) => c.status === 'done').length,
        total: children.filter((c) => c.status !== 'canceled').length,
      },
      activity_total: t.activity.length,
    }
  }

  private record(t: FakeTicket, type: string, actor: string, at: string, changes: Payload[] = [], comment?: string) {
    t.activity.push({
      id: `a${this.seq}`,
      ticket_id: t.id,
      seq: this.seq++,
      type,
      actor,
      at,
      ...(changes.length ? { changes } : {}),
      ...(comment ? { comment_id: comment } : {}),
    })
  }

  /** Deliveries in flight (`elsewhere` waits for them). */
  private sending: Promise<unknown>[] = []

  /** The `kanban:change` of a write, sent to the host when one is attached. */
  emit(event: string, t: FakeTicket, at = t.updated_at): Promise<void> {
    if (!this.host) return Promise.resolve()
    const sent = this.host.emitTrigger('kanban:change', {
      event,
      ticket_id: t.id,
      ticket_key: t.key,
      ticket: this.summary(t),
      comment: null,
      activity: t.activity[t.activity.length - 1] ?? null,
      at,
    })
    this.sending.push(sent)
    return sent.then(() => {})
  }

  /** A write made by someone else (an agent, the console): the worker's answer, its events delivered. */
  async elsewhere(fn: string, payload: Payload): Promise<unknown> {
    const result = this.answer(`kanban::${fn}`, { actor: 'backend-engineer', ...payload })
    await Promise.all(this.sending.splice(0))
    await this.host?.settle()
    return result
  }

  hold(fn: string): void {
    this.holding.add(`kanban::${fn}`)
  }

  waiting(fn: string): Held[] {
    return this.held.get(`kanban::${fn}`) ?? []
  }

  release(fn: string, value?: unknown): void {
    const id = `kanban::${fn}`
    this.holding.delete(id)
    const h = this.held.get(id)?.shift()
    if (h) h.resolve(value === undefined ? this.answer(id, h.payload) : value)
  }

  calls(): TestHostOptions['calls'] {
    const ids = [
      'config::info',
      'board::get',
      'ticket::list',
      'ticket::get',
      'ticket::links',
      'ticket::create',
      'ticket::update',
      'ticket::move',
      'ticket::delete',
      'ticket::restore',
      'comment::create',
      'agent::list',
      'project::list',
      'release::list',
    ]
    const out: Record<string, (payload: any) => unknown> = {}
    for (const id of ids) {
      const fn = `kanban::${id}`
      out[fn] = (payload) => {
        const p = (payload ?? {}) as Payload
        if (this.holding.has(fn)) {
          return new Promise((resolve, reject) => {
            const list = this.held.get(fn) ?? []
            list.push({ resolve, reject, payload: p })
            this.held.set(fn, list)
          })
        }
        return this.answer(fn, p)
      }
    }
    return out
  }

  private page<T>(rows: T[], p: Payload): T[] {
    const from = p.offset ?? 0
    return p.limit ? rows.slice(from, from + p.limit) : rows.slice(from)
  }

  private plan() {
    const live = this.tickets.filter((t) => !t.deleted_at)
    const tally = (keep: (t: FakeTicket) => boolean) => {
      const mine = live.filter(keep)
      return {
        total: mine.filter((t) => t.status !== 'canceled').length,
        done: mine.filter((t) => t.status === 'done').length,
      }
    }
    return {
      projects: this.projects.map((p) => ({
        ...p,
        status: p.archived_at ? 'archived' : 'active',
        ...tally((t) => t.project_id === p.id),
      })),
      releases: this.releases.map((r) => ({
        ...r,
        status: r.released_at ? 'released' : 'planned',
        ...tally((t) => t.release_id === r.id),
      })),
    }
  }

  answer(fn: string, p: Payload): unknown {
    const name = fn.slice('kanban::'.length)
    const override = this.overrides[name]
    if (override) return override(p)
    const fail = (message: string) => {
      throw new HostError('invocation_failed', message)
    }
    const found = (ref: string) => this.get(String(ref ?? '')) ?? fail(`ticket not found: ${ref}`)
    switch (name) {
      case 'config::info':
        return {
          columns: this.columns,
          priorities: this.priorities,
          default_status: 'backlog',
          done_status: 'done',
          canceled_status: 'canceled',
          id_prefix: this.prefix,
        }
      case 'board::get':
        return {
          columns: this.columns.map((c) => {
            const rows = this.column(c.id)
            return {
              id: c.id,
              label: c.label,
              tickets: this.page(rows, p).map((t) => this.summary(t)),
              total: rows.length,
            }
          }),
          ticket_count: this.tickets.filter((t) => !t.deleted_at).length,
          deleted_count: this.tickets.filter((t) => t.deleted_at).length,
          priorities: this.priorities,
          default_status: 'backlog',
          done_status: 'done',
          canceled_status: 'canceled',
          id_prefix: this.prefix,
          updated_at: new Date(Math.max(Date.now(), this.clock)).toISOString(),
          ...this.plan(),
        }
      case 'ticket::list': {
        const rows = this.column(p.status)
        const tickets = this.page(rows, p).map((t) => this.summary(t))
        return { tickets, count: tickets.length, total: rows.length }
      }
      case 'ticket::get':
        return this.view(found(p.id))
      case 'ticket::links': {
        const t = found(p.id)
        const keys = (text: string) => (text.match(/KAN-\d+/g) ?? []).map((k) => k.toUpperCase())
        const own = new Set([t.title, t.description, ...t.comments.map((c) => c.body)].flatMap(keys))
        const live = this.tickets.filter((x) => !x.deleted_at && x.id !== t.id)
        return {
          mentions: live.filter((x) => own.has(x.key)).map((x) => this.summary(x)),
          mentioned_by: live
            .filter((x) => [x.title, x.description, ...x.comments.map((c) => c.body)].flatMap(keys).includes(t.key))
            .map((x) => this.summary(x)),
        }
      }
      case 'ticket::create': {
        if (!String(p.title ?? '').trim()) fail('title is required')
        const t = this.add({
          title: p.title,
          description: p.description ?? '',
          status: p.status ?? 'backlog',
          priority: p.priority ?? 'low',
          assignee: p.assignee ?? null,
          labels: p.labels ?? [],
          parent_id: p.parent ? found(p.parent).id : null,
          project_id: p.project ?? null,
          release_id: p.release ?? null,
          created_by: p.actor ?? 'user',
        })
        void this.emit('ticket.created', t)
        return this.view(t)
      }
      case 'ticket::update': {
        const t = found(p.id)
        const at = this.stamp()
        const changes: Payload[] = []
        const set = (field: keyof FakeTicket, value: unknown, key = field as string) => {
          if (JSON.stringify(t[field]) === JSON.stringify(value)) return
          changes.push({ field: key, from: t[field] ?? null, to: value ?? null })
          ;(t as any)[field] = value
        }
        if ('title' in p) set('title', p.title)
        if ('description' in p) set('description', p.description)
        if ('priority' in p) set('priority', p.priority)
        if ('assignee' in p) set('assignee', p.assignee)
        if ('status' in p) {
          if (t.deleted_at) fail('cannot change the status of a deleted ticket')
          if (p.status !== t.status) {
            const order = this.max(p.status) + 1
            set('status', p.status)
            t.order = order
          }
        }
        if ('labels' in p) set('labels', p.labels)
        if (p.remove_labels || p.add_labels) {
          let labels = t.labels.filter(
            (l) => !(p.remove_labels ?? []).some((r: string) => r.toLowerCase() === l.toLowerCase()),
          )
          for (const l of p.add_labels ?? [])
            if (!labels.some((x) => x.toLowerCase() === l.toLowerCase())) labels = [...labels, l]
          set('labels', labels)
        }
        if ('parent' in p) set('parent_id', p.parent === null ? null : found(p.parent).id, 'parent')
        if ('project' in p) set('project_id', p.project, 'project')
        if ('release' in p) set('release_id', p.release, 'release')
        if (changes.length) {
          t.updated_at = at
          this.record(t, 'ticket.updated', p.actor ?? 'user', at, changes)
          void this.emit('ticket.updated', t)
        }
        return this.view(t)
      }
      case 'ticket::move': {
        const t = found(p.id)
        if (t.deleted_at) fail('cannot move a deleted ticket')
        if (!this.columns.some((c) => c.id === p.status)) fail(`unknown status: ${p.status}`)
        const previous = t.status
        const column = this.column(p.status)
        const ordered = column.filter((x) => x.id !== t.id)
        const at = Math.min(p.index ?? ordered.length, ordered.length)
        ordered.splice(at, 0, t)
        if (previous === p.status && column.every((x, i) => x.id === ordered[i].id)) return this.view(t)
        ordered.forEach((x, i) => {
          x.order = i + 1
        })
        const stamp = this.stamp()
        t.status = p.status
        t.updated_at = stamp
        if (previous !== p.status) {
          this.column(previous).forEach((x, i) => {
            x.order = i + 1
          })
          this.record(t, 'ticket.moved', p.actor ?? 'user', stamp, [{ field: 'status', from: previous, to: p.status }])
        }
        void this.emit('ticket.moved', t)
        return this.view(t)
      }
      case 'ticket::delete':
      case 'ticket::restore': {
        const t = found(p.id)
        const deleting = name === 'ticket::delete'
        if (deleting === !!t.deleted_at) return this.view(t)
        const at = this.stamp()
        t.deleted_at = deleting ? at : null
        t.updated_at = at
        this.record(t, deleting ? 'ticket.deleted' : 'ticket.restored', p.actor ?? 'user', at)
        void this.emit(deleting ? 'ticket.deleted' : 'ticket.restored', t)
        return this.view(t)
      }
      case 'comment::create': {
        const t = found(p.ticket_id)
        if (!String(p.body ?? '').trim()) fail('body is required')
        const at = this.stamp()
        let parent = p.parent_id ? t.comments.find((c) => c.id === p.parent_id) : undefined
        if (p.parent_id && !parent) fail(`comment not found: ${p.parent_id}`)
        if (parent?.parent_id) parent = t.comments.find((c) => c.id === parent?.parent_id)
        const comment = {
          id: `c${this.seq}`,
          ticket_id: t.id,
          parent_id: parent?.id ?? null,
          body: p.body,
          author: p.author ?? 'user',
          created_at: at,
        }
        t.comments.push(comment)
        t.updated_at = at
        this.record(t, 'comment.created', comment.author, at, [], comment.id)
        void this.emit('comment.created', t)
        return { comment, ticket: this.view(t) }
      }
      case 'agent::list':
        return { source: 'directory', agents: this.agents }
      case 'project::list':
        return { projects: this.plan().projects }
      case 'release::list':
        return { releases: this.plan().releases }
      default:
        throw new HostError('function_not_found', `Function ${fn} not found`)
    }
  }
}

/**
 * What the person answers, in order: `menus` are the labels picked in the
 * next action menus (a label the menu lacks, or none left, dismisses it),
 * `confirms` the next confirmations (none left = Cancel).
 */
export class Answers {
  menus: string[] = []
  confirms: boolean[] = []
  ui(): NonNullable<TestHostOptions['ui']> {
    return {
      actionMenu: (args: any) => {
        const label = this.menus.shift()
        return label === undefined ? -1 : (args.actions as { label: string }[]).findIndex((a) => a.label === label)
      },
      confirm: () => this.confirms.shift() ?? false,
    }
  }
}

export interface Started {
  host: TestHost
  fake: FakeKanban
  answers: Answers
}

/** A host running the app's bundle against `fake`, with the root page mounted and the board read. */
export async function start(
  options: {
    fake?: FakeKanban
    env?: Partial<Env>
    ui?: TestHostOptions['ui']
    storage?: TestHostOptions['storage']
    mount?: boolean
    /** Send the writes' `kanban:change` events to the host (default true). */
    live?: boolean
  } = {},
): Promise<Started> {
  const fake = options.fake ?? sample()
  const answers = new Answers()
  const host = await createTestHost({
    bundle: inject('bundle'),
    ns: 'kanban',
    calls: fake.calls(),
    env: options.env,
    ui: options.ui ?? answers.ui(),
    storage: options.storage,
  })
  if (options.live !== false) fake.host = host
  if (options.mount !== false) {
    await host.mount('root')
    await host.settle()
  }
  return { host, fake, answers }
}

/** The node `target` on the top card (a page pushed over the root has its own `ticket`, `composer`…). */
export function onTop(host: TestHost, target: string | Record<string, unknown>): number {
  const inst = host.deck.at(-1)?.inst
  const root = inst === undefined ? undefined : host.roots.get(inst)
  const found = root === undefined ? [] : host.findAll(target as any, { within: root })
  if (found.length !== 1) throw new Error(`${found.length} nodes match ${JSON.stringify(target)} on the top card`)
  return found[0].id
}

/** The name and params of the top card. */
export const topCard = (host: TestHost) => host.deck.at(-1)

/** A small board: a parent with subtasks, labels, pills, comments with a reply, and a link in the text. */
export function sample(): FakeKanban {
  const f = new FakeKanban()
  const parent = f.add({
    title: 'Search v2',
    status: 'in_progress',
    priority: 'high',
    assignee: 'tech-lead',
    labels: ['search', 'frontend', 'backend', 'ux'],
    project_id: 'p1',
    release_id: 'r1',
    description: 'The search epic. See KAN-4 for the index.',
  })
  f.add({
    title: 'Persist filters in the URL',
    status: 'todo',
    parent_id: parent.id,
    assignee: 'frontend-engineer',
    labels: ['frontend'],
  })
  f.add({ title: 'Index titles', status: 'done', parent_id: parent.id, assignee: 'backend-engineer' })
  const review = f.add({
    title: 'Search index',
    status: 'in_review',
    priority: 'urgent',
    assignee: 'backend-engineer',
    pills: [
      { icon: 'calendar', text: 'Fri 17:00' },
      { icon: 'no-such-icon', text: 'Prod' },
    ],
    description: '## Done\n\n- Built it\n- Tested it, see [the run](https://ci.example.com/run/7)\n\nPart of KAN-1.',
  })
  f.add({ title: 'Write the docs', status: 'backlog', priority: 'low' })
  const at = f.stamp()
  review.comments.push(
    {
      id: 'c1',
      ticket_id: review.id,
      parent_id: null,
      body: 'Pushed the fix; ready for review.',
      author: 'backend-engineer',
      created_at: at,
    },
    {
      id: 'c2',
      ticket_id: review.id,
      parent_id: 'c1',
      body: 'Looks good, **merging**.',
      author: 'user',
      created_at: at,
    },
  )
  review.activity.push(
    {
      id: 'ac1',
      ticket_id: review.id,
      seq: f.seq++,
      type: 'comment.created',
      actor: 'backend-engineer',
      at,
      comment_id: 'c1',
    },
    { id: 'ac2', ticket_id: review.id, seq: f.seq++, type: 'comment.created', actor: 'user', at, comment_id: 'c2' },
  )
  review.updated_at = at
  return f
}

/** The payloads of every `call` of `fn` (without `kanban::`), in order. */
export function callsOf(host: TestHost, fn: string): Payload[] {
  return host.requests
    .filter((r) => r.method === 'call' && r.args.fn === `kanban::${fn}`)
    .map((r) => (r.args.payload ?? {}) as Payload)
}

/** The cards of a lane as the host holds them. */
export function cards(host: TestHost, lane: string): Payload[] {
  return host.items(`lane:${lane}`) as Payload[]
}

export const keysOf = (host: TestHost, lane: string) => cards(host, lane).map((c) => c.ticketKey)

export function shown(host: TestHost, testTag: string): boolean {
  const all = host.findAll(testTag)
  return all.length > 0 && all.every((n) => n.props?.visible !== false)
}
