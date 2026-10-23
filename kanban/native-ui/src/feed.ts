/**
 * Cards for the lanes' Lists without redoing the board on every change
 * (design §8.2): a card row is cached by its ticket, so a write that touches
 * one ticket computes one row, and a lane whose rows did not change sends
 * nothing; a lane that grows by thousands (the whole board read for the
 * filters) is fed in steps, each a VM turn of its own, so no step comes near
 * the VM's deadline.
 */

import { type createList, untrack } from '@iii-dev/native-ui'
import type { Summary } from './api'
import { type CardContext, type CardRow, cardRow } from './cards'

type Source = ReturnType<typeof createList<CardRow>>

/** Rows added per step when a lane grows a lot at once. */
export const CHUNK = 300

interface Entry {
  ticket: Summary
  base: object
  parent: string | undefined
  subs: string
  selected: boolean
  row: CardRow
}

/** `rowOf(ticket, ctx, base)`: the cached row while the ticket, `base` (the shared inputs) and its own lookups stay the same. */
export function createCardCache(): (ticket: Summary, ctx: CardContext, base: object) => CardRow {
  const cache = new Map<string, Entry>()
  return (ticket, ctx, base) => {
    const parent = ticket.parent_id ? ctx.keys.get(ticket.parent_id) : undefined
    const stats = ctx.subs?.get(ticket.id)
    const subs = stats ? `${stats.done}/${stats.total}` : ''
    const selected = ctx.selected === ticket.key
    const hit = cache.get(ticket.id)
    if (
      hit &&
      hit.ticket === ticket &&
      hit.base === base &&
      hit.parent === parent &&
      hit.subs === subs &&
      hit.selected === selected
    )
      return hit.row
    const row = cardRow(ticket, ctx)
    cache.set(ticket.id, { ticket, base, parent, subs, selected, row })
    return row
  }
}

/** `feed(source, items)`: nothing when the items are the ones it has, else a set, or a big growth in steps. */
export function createFeeder(): (source: Source, items: CardRow[]) => void {
  const timers = new Map<Source, number>()
  return (source, items) => {
    const timer = timers.get(source)
    if (timer !== undefined) {
      clearTimeout(timer)
      timers.delete(source)
    }
    const current = untrack(() => source.items())
    if (current.length === items.length && current.every((row, i) => row === items[i])) return
    if (items.length - current.length <= CHUNK) {
      source.set(items)
      return
    }
    let shown = Math.max(CHUNK, current.length)
    source.set(items.slice(0, shown))
    const step = () => {
      const next = Math.min(items.length, shown + CHUNK)
      source.splice(shown, 0, ...items.slice(shown, next))
      shown = next
      if (shown < items.length) timers.set(source, setTimeout(step, 0))
      else timers.delete(source)
    }
    timers.set(source, setTimeout(step, 0))
  }
}
