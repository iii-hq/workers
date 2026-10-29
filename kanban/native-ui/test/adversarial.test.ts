/**
 * What breaks apps on a device (SDK README › Pitfalls), tried on purpose:
 * malformed worker data, calls that hang, the engine going away, drops and
 * links that make no sense, hostile text, a big board, garbage in storage.
 * None may throw in the VM, trip its step deadline, or leave a spinner on.
 */
import { HostError } from '@iii-dev/native-ui/testing'
import { describe, expect, it } from 'vitest'
import {
  activityOf,
  callsOf,
  cards,
  descriptionOf,
  FakeKanban,
  itemOf,
  keysOf,
  piecesOf,
  type Started,
  sample,
  start,
  tapLink,
} from './fake'

describe('malformed worker data', () => {
  it('a board with bad columns and bad cards shows the good ones', async () => {
    const fake = sample()
    fake.overrides['board::get'] = () => ({
      columns: [
        {
          id: 'todo',
          label: 'To do',
          total: 'many',
          tickets: [
            {
              id: 'a',
              key: 'KAN-1',
              status: 'todo',
              title: 42,
              labels: [1, 'ok', null],
              pills: [{ icon: 3 }, { text: 'Pill' }, 'x'],
            },
            { id: 'a', key: 'KAN-1', status: 'todo', title: 'Again' },
            { key: 'no id' },
            7,
          ],
        },
        { id: 'doing', label: null, tickets: 'nope' },
        { label: 'no id' },
        {
          id: 'done',
          label: 'Done',
          tickets: [
            { id: 'a', key: 'KAN-1', status: 'done' },
            { id: 'w', key: 'KAN-9', status: 'todo' },
          ],
        },
      ],
      priorities: [1, 'high'],
      projects: 'x',
      releases: [null, { id: 'r', name: 5 }],
    })
    const { host } = await start({ fake })
    expect(host.failed).toBeNull()
    expect(host.lanes('lanes').titles).toEqual(['To do', 'doing', 'Done'])
    // The repeat keeps its last version; a card in two columns stays in the first; one whose status is not its column is out.
    expect(cards(host, 'todo')).toHaveLength(1)
    expect(cards(host, 'todo')[0]).toMatchObject({ title: 'Again' })
    expect(cards(host, 'done')).toEqual([])
    expect(host.lanes('lanes').counts).toEqual(['1', '0', '0'])
  })

  it('a ticket with broken parts still opens', async () => {
    const fake = sample()
    fake.overrides['ticket::get'] = () => ({
      id: 't4',
      key: 'KAN-4',
      status: 'in_review',
      title: null,
      description: { not: 'text' },
      comments: [{ id: 'c1', body: 7, parent_id: 'gone' }, 'bad', { id: 'c1', body: 'dup' }],
      activity: [
        { id: 'x', type: 'ticket.updated', changes: 'no' },
        { id: 'y', type: 'comment.created', comment_id: 'c1' },
        {},
      ],
      parent: 'KAN-1',
      children: [{ key: 'no id' }],
      progress: { done: 'a' },
      labels: 'x',
    })
    fake.overrides['ticket::links'] = () => ({ mentions: 'no', mentioned_by: [null, { id: 'z' }] })
    const { host } = await start({ fake })
    await host.itemPress('lane:in_review', 't4')
    await host.settle()
    expect(host.failed).toBeNull()
    expect(host.prop('title', 'text')).toBe('KAN-4')
    expect(piecesOf(host)).toEqual([])
    expect(itemOf(host, '@d-none')).toMatchObject({ isNote: true, note: 'No description yet.' })
    expect(activityOf(host).map((i) => [i.kind, i.text])).toEqual([
      [0, 'You updated this'],
      [1, 'dup'],
    ])
  })

  it('garbage events change nothing on the board, but read it again once after the burst', async () => {
    const fake = sample()
    fake.overrides['agent::list'] = () => 'agents'
    const { host } = await start({ fake })
    for (const payload of [
      null,
      'x',
      1,
      [],
      { event: 7 },
      { event: 'ticket.moved' },
      { event: 'ticket.moved', ticket: { id: 't1' } },
      { event: 'ticket.moved', ticket: 'KAN-1' },
    ]) {
      await host.emitTrigger('kanban:change', payload as never)
    }
    await host.settle()
    expect(host.failed).toBeNull()
    expect(host.exceptions).toEqual([])
    expect(keysOf(host, 'in_progress')).toEqual(['KAN-1'])
    expect(cards(host, 'in_progress')[0]).toMatchObject({ assigneeIcon: 'bot' })
    // Something changed that the app could not read: one quiet read, 400 ms after the last.
    const reads = callsOf(host, 'board::get').length
    await host.tick(400)
    expect(callsOf(host, 'board::get')).toHaveLength(reads + 1)
  })

  it('comments in a parent cycle, their own parent, or replying to a reply are all listed', async () => {
    const fake = sample()
    const review = fake.get('KAN-4')
    if (!review) throw new Error('no KAN-4')
    const at = fake.stamp()
    review.comments.push(
      { id: 'self', ticket_id: 't4', parent_id: 'self', body: 'self-parent', author: 'user', created_at: at },
      { id: 'cx', ticket_id: 't4', parent_id: 'cy', body: 'cycle-x', author: 'user', created_at: at },
      { id: 'cy', ticket_id: 't4', parent_id: 'cx', body: 'cycle-y', author: 'user', created_at: at },
      { id: 'deep', ticket_id: 't4', parent_id: 'c2', body: 'reply-to-a-reply', author: 'user', created_at: at },
    )
    review.activity.push(
      { id: 'aself', ticket_id: 't4', seq: fake.seq++, type: 'comment.created', actor: 'user', at, comment_id: 'self' },
      { id: 'adeep', ticket_id: 't4', seq: fake.seq++, type: 'comment.created', actor: 'user', at, comment_id: 'deep' },
    )
    const { host } = await start({ fake })
    await host.itemPress('lane:in_review', 't4')
    await host.settle()
    const rows = activityOf(host).filter((r) => r.kind === 1)
    expect(rows.map((r) => [r.text, r.isReply])).toEqual([
      ['Pushed the fix; ready for review.', false],
      ['Looks good, **merging**.', true],
      // The reply to a reply goes under the same root, and replying to it answers that root.
      ['reply-to-a-reply', true],
      ['self-parent', false],
      ['cycle-x', false],
      ['cycle-y', false],
    ])
    expect(itemOf(host, 'c:deep')).toMatchObject({ root: 'c1' })
  })

  it('a card that moves to a column the app does not have leaves the board and its count', async () => {
    const { host, fake } = await start({ live: false })
    const t = fake.get('KAN-5')
    if (!t) throw new Error('no KAN-5')
    t.status = 'ghost'
    t.updated_at = fake.stamp()
    await host.emitTrigger('kanban:change', { event: 'ticket.updated', ticket: fake.summary(t), at: t.updated_at })
    await host.settle()
    expect(keysOf(host, 'backlog')).toEqual([])
    expect(host.lanes('lanes').counts[0]).toBe('0')
  })

  it('storage holding garbage reads as the defaults', async () => {
    const { host } = await start({ storage: { filters: 'labels=x', lane: 42, 'visit:KAN': { when: 'yesterday' } } })
    expect(host.failed).toBeNull()
    expect(host.prop('filters', 'active')).not.toBe(true)
    expect(host.lanes('lanes').selected).toBe(0)
    expect(cards(host, 'todo')[0]).toMatchObject({ changed: false })
  })
})

describe('calls that hang', () => {
  it('a board read that never answers clears at the timeout and says so', async () => {
    const fake = sample()
    fake.hold('board::get')
    const { host } = await start({ fake })
    expect(host.findAll('board-loading')).toHaveLength(1)
    await host.tick(30_000)
    await host.settle()
    expect(host.find('board-error').props?.detail).toBe("The engine didn't answer in time")
    expect(host.findAll('board-loading')).toHaveLength(0)
  })

  it('a ticket read that never answers ends in the error with Retry', async () => {
    const { host, fake } = await start()
    fake.hold('ticket::get')
    await host.itemPress('lane:in_review', 't4')
    expect(host.findAll('ticket-loading')).toHaveLength(1)
    await host.tick(30_000)
    await host.settle()
    expect(host.find('ticket-error').props).toMatchObject({ description: "The engine didn't answer in time" })
    fake.release('ticket::get')
    await host.press('ticket-retry')
    await host.settle()
    expect(host.prop('title', 'text')).toBe('Search index')
  })

  it('a comment that never lands gives the send button back and keeps the text', async () => {
    const { host, fake } = await start()
    await host.itemPress('lane:in_review', 't4')
    await host.settle()
    fake.hold('comment::create')
    await host.type('composer', 'Still there?')
    await host.press('send')
    expect(host.findAll('sending')).toHaveLength(1)
    await host.tick(30_000)
    await host.settle()
    expect(host.findAll('sending')).toHaveLength(0)
    expect(host.findAll('send')).toHaveLength(1)
    expect(host.prop('composer', 'value')).toBe('Still there?')
  })
})

describe('offline', () => {
  it('turns off what calls the worker, and reads again when the engine is back', async () => {
    const { host } = await start()
    await host.itemPress('lane:in_review', 't4')
    await host.settle()
    await host.env({ online: false })
    expect(host.prop('new-ticket', 'enabled')).toBe(false)
    expect(host.prop('status', 'enabled')).toBe(false)
    expect(host.prop('edit-description', 'enabled')).toBe(false)
    await host.type('composer', 'queued?')
    expect(host.prop('send', 'enabled')).toBe(false)
    const reads = callsOf(host, 'board::get').length
    await host.env({ online: true })
    await host.settle()
    expect(callsOf(host, 'board::get').length).toBe(reads + 1)
    expect(host.prop('send', 'enabled')).not.toBe(false)
  })
})

describe('drops and links that make no sense', () => {
  it('a drop of an unknown card, or from a list that is gone, does nothing', async () => {
    const { host } = await start()
    await host.emit('lane:todo', 'drop', ['nope', 0, host.find('lane:done').id])
    await host.emit('lane:todo', 'drop', ['t5', 0, 999_999])
    await host.emit('lane:todo', 'drop', ['t5', -4, host.find('lane:backlog').id])
    await host.settle()
    expect(host.failed).toBeNull()
    // The last one is a real card from a real list: a negative index is the top.
    expect(callsOf(host, 'ticket::move')).toEqual([{ id: 't5', status: 'todo', index: 0, actor: 'user' }])
  })

  it('an unknown ticket key, a script link or a bare word is refused in words', async () => {
    const fake = sample()
    const review = fake.get('KAN-4')
    if (review)
      review.description =
        'Bad: [run](javascript:alert(1)) [file](file:///etc/passwd) [ghost](#kanban-ticket/KAN-999) KAN-1'
    const { host } = await start({ fake })
    await host.itemPress('lane:in_review', 't4')
    await host.settle()
    for (const href of ['javascript:alert(1)', 'file:///etc/passwd', '#kanban-ticket/KAN-999']) {
      await tapLink(host, href)
      expect(host.toasts.at(-1)).toEqual({ text: "This link can't be opened here", kind: 'info' })
    }
    expect(host.opened).toEqual([])
    expect(host.deck.map((c) => c.name)).toEqual(['root'])
  })
})

describe('size', () => {
  it('a description at the Markdown limit with keys in it opens within the step deadline', async () => {
    const fake = sample()
    const review = fake.get('KAN-4')
    const chunk = 'Line about KAN-1 and `code` and **bold** text, see https://x.example/y.\n'
    if (review) review.description = chunk.repeat(Math.floor(190_000 / chunk.length))
    const { host } = await start({ fake })
    await host.itemPress('lane:in_review', 't4')
    await host.settle()
    expect(host.failed).toBeNull()
    expect(descriptionOf(host).length).toBeGreaterThan(180_000)
    expect(piecesOf(host).length).toBeGreaterThan(100)
  })

  it('a ticket with 600 comments lists them', async () => {
    const fake = sample()
    const review = fake.get('KAN-4')
    if (!review) throw new Error('no KAN-4')
    for (let i = 0; i < 600; i++) {
      const at = fake.stamp()
      review.comments.push({
        id: `x${i}`,
        ticket_id: 't4',
        parent_id: i % 3 ? 'c1' : null,
        body: `Note ${i} on KAN-1`,
        author: 'tech-lead',
        created_at: at,
      })
      review.activity.push({
        id: `ax${i}`,
        ticket_id: 't4',
        seq: fake.seq++,
        type: 'comment.created',
        actor: 'tech-lead',
        at,
        comment_id: `x${i}`,
      })
    }
    const { host } = await start({ fake })
    await host.itemPress('lane:in_review', 't4')
    await host.settle()
    expect(host.failed).toBeNull()
    expect(activityOf(host)).toHaveLength(603)
  })

  it('a board with 3 000 done tickets: the first page, then the whole of it under a filter', async () => {
    const fake = sample()
    for (let i = 0; i < 3000; i++)
      fake.add({
        title: `Finished task number ${i}`,
        status: 'done',
        labels: ['bulk', `l${i % 7}`],
        assignee: 'backend-engineer',
      })
    const { host } = await start({ fake, storage: { 'visit:KAN': new Date(0).toISOString() } })
    expect(cards(host, 'done')).toHaveLength(50)
    expect(cards(host, 'done')[0]).toMatchObject({ changed: true })
    expect(host.lanes('lanes').counts[4]).toBe('3k')
    await host.press('filters')
    await host.settle()
    expect(host.failed).toBeNull()
    // The lane grows in steps of 300, a turn each (1 ms apart).
    expect(cards(host, 'done')).toHaveLength(300)
    await host.tick(20)
    await host.back()
    await host.settle()
    // A step that passes the VM's deadline inside a promise job stops it silently in the test host
    // (the rest of the step never runs): the whole lane being here is the check.
    expect(cards(host, 'done')).toHaveLength(3001)
    expect(host.failed).toBeNull()
    // A change on the whole board computes one card, not three thousand.
    await fake.elsewhere('ticket::move', { id: 'KAN-100', status: 'todo', index: 0 })
    expect(cards(host, 'done')).toHaveLength(3000)
    expect(keysOf(host, 'todo')).toEqual(['KAN-100', 'KAN-2'])
  })
})

describe('reads answered before a write and delivered after it', () => {
  /** Holds the next board read, lets `write` happen, then delivers the read as the worker answered it before. */
  async function staleRead(fake: FakeKanban, host: Started['host'], lane: string, write: () => Promise<unknown>) {
    fake.hold('board::get')
    await host.emit(`lane:${lane}`, 'refresh')
    await host.settle()
    const snapshot = fake.answer('kanban::board::get', fake.waiting('board::get')[0].payload)
    await write()
    fake.release('board::get', snapshot)
    await host.settle()
  }

  it('a ticket created during the read stays, counted once', async () => {
    const fake = new FakeKanban()
    for (let i = 0; i < 3; i++) fake.add({ title: `D${i}`, status: 'done' })
    const { host } = await start({ fake })
    await staleRead(fake, host, 'done', () => fake.elsewhere('ticket::create', { title: 'NEW', status: 'done' }))
    expect(host.lanes('lanes').counts[4]).toBe('4')
    expect(keysOf(host, 'done')).toEqual(fake.column('done').map((t) => t.key))
  })

  it('a ticket restored during the read is listed', async () => {
    const fake = new FakeKanban()
    const gone = fake.add({ title: 'Gone then back', status: 'todo' })
    fake.answer('kanban::ticket::delete', { id: gone.id })
    const { host } = await start({ fake })
    await staleRead(fake, host, 'todo', () => fake.elsewhere('ticket::restore', { id: gone.id }))
    expect(host.lanes('lanes').counts[1]).toBe('1')
    expect(keysOf(host, 'todo')).toEqual([gone.key])
  })

  it('a ticket from past the first page moved in during the read is drawn and counted', async () => {
    const fake = new FakeKanban()
    for (let i = 0; i < 3; i++) fake.add({ title: `D${i}`, status: 'done' })
    for (let i = 0; i < 60; i++) fake.add({ title: `B${i}`, status: 'backlog' })
    const far = fake.column('backlog')[59]
    const { host } = await start({ fake })
    await staleRead(fake, host, 'done', () => fake.elsewhere('ticket::move', { id: far.id, status: 'done', index: 0 }))
    expect(keysOf(host, 'done')).toEqual(fake.column('done').map((t) => t.key))
    expect(host.lanes('lanes').counts[4]).toBe('4')
  })

  /** xorshift32: the same sequence on every run. */
  function rng(seed: number) {
    let s = seed >>> 0 || 1
    return () => {
      s ^= s << 13
      s >>>= 0
      s ^= s >>> 17
      s ^= s << 5
      s >>>= 0
      return s / 4294967296
    }
  }
  const COLS = ['backlog', 'todo', 'in_progress', 'in_review', 'done']

  // The review's fuzz: reads and pages held while writes land, then delivered stale; after each step every lane
  // shows the start of its worker column, in order, and its count.
  for (const seed of Array.from({ length: 25 }, (_, i) => i + 1)) {
    it(`fuzz seed ${seed}`, async () => {
      const r = rng(seed * 7717)
      const fake = new FakeKanban()
      for (let i = 0; i < 90; i++) fake.add({ title: `S${i}`, status: COLS[Math.floor(r() * COLS.length)] })
      const { host } = await start({ fake })
      const log: string[] = []
      const check = (step: string) => {
        for (const col of COLS) {
          const shown = cards(host, col).map((c) => c.key as string)
          const truth = fake.column(col).map((t) => t.id)
          const count = host.lanes('lanes').counts[COLS.indexOf(col)]
          const ok =
            JSON.stringify(shown) === JSON.stringify(truth.slice(0, shown.length)) &&
            shown.length <= truth.length &&
            String(count) === String(truth.length)
          if (!ok)
            throw new Error(`[${step}] ${col}: shown ${shown} (${count}); worker ${truth}\n${log.slice(-8).join('\n')}`)
        }
      }
      const live = () => {
        const all = fake.tickets.filter((t) => !t.deleted_at)
        return all[Math.floor(r() * all.length)]
      }
      for (let step = 0; step < 60; step++) {
        const col = COLS[Math.floor(r() * COLS.length)]
        if (r() < 0.5) {
          const board = r() < 0.5
          const fn = board ? 'board::get' : 'ticket::list'
          fake.hold(fn)
          log.push(board ? `refresh ${col}` : `endReached ${col}`)
          await host.emit(`lane:${col}`, board ? 'refresh' : 'endReached')
          await host.settle()
          const snapshots = fake.waiting(fn).map((h) => fake.answer(`kanban::${fn}`, h.payload))
          for (let w = 1 + Math.floor(r() * 4); w > 0; w--) {
            const t = live()
            const to = COLS[Math.floor(r() * COLS.length)]
            const op = r()
            log.push(`${op < 0.5 ? 'move' : op < 0.7 ? 'create' : op < 0.85 ? 'delete' : 'update'} ${t.key} ${to}`)
            if (op < 0.5) await fake.elsewhere('ticket::move', { id: t.id, status: to, index: Math.floor(r() * 3) })
            else if (op < 0.7) await fake.elsewhere('ticket::create', { title: `W${step}`, status: to })
            else if (op < 0.85) await fake.elsewhere('ticket::delete', { id: t.id })
            else await fake.elsewhere('ticket::update', { id: t.id, status: to })
          }
          for (const snapshot of snapshots) fake.release(fn, snapshot)
          while (fake.waiting(fn).length) fake.release(fn)
          await host.settle()
        } else {
          const t = live()
          const to = COLS[Math.floor(r() * COLS.length)]
          log.push(`move ${t.key} ${to}`)
          await fake.elsewhere('ticket::move', { id: t.id, status: to, index: Math.floor(r() * 3) })
          await host.settle()
        }
        check(`seed ${seed} step ${step}`)
      }
      expect(host.failed).toBeNull()
    })
  }
})

describe('the worker refusing', () => {
  it('a write the worker refuses says why and reads the ticket again', async () => {
    const fake = sample()
    fake.overrides['ticket::update'] = () => {
      throw new HostError('invocation_failed', 'project Old site is archived; it takes no new tickets')
    }
    const { host, answers } = await start({ fake })
    await host.itemPress('lane:in_review', 't4')
    await host.settle()
    answers.menus.push('Low')
    await host.press('priority')
    await host.settle()
    expect(host.toasts.at(-1)).toEqual({
      text: "Couldn't save: project Old site is archived; it takes no new tickets",
      kind: 'error',
    })
    expect(callsOf(host, 'ticket::get')).toHaveLength(2)
    expect(host.find('priority').props).toMatchObject({ text: 'Urgent' })
  })

  it('an empty board is a board', async () => {
    const { host } = await start({ fake: new FakeKanban() })
    expect(host.tree()?.props).toMatchObject({ subtitle: '0 tickets' })
    expect(host.findAll({ type: 'EmptyState', title: 'No tickets in Backlog' })).toHaveLength(1)
  })
})
