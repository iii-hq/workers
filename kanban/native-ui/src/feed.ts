/**
 * Cards for the lanes' Lists without redoing the board on every change
 * (design §8.2): a card row is cached by its ticket, so a write that touches
 * one ticket computes one row, and a lane whose rows did not change sends
 * nothing; a lane that grows by thousands (the whole board read for the
 * filters) is fed in steps, each a VM turn of its own, so no step comes near
 * the VM's deadline.
 */

import { type createList, untrack } from '@iii-dev/native-ui'
import type { CardRow } from './cards'

type Source = ReturnType<typeof createList<CardRow>>

/** Rows added per step when a lane grows a lot at once. */
export const CHUNK = 300
/** The pause between steps (ms): each step is a turn of its own, and a touch can come in between. */
export const FEED_PAUSE = 1

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
      if (shown < items.length) timers.set(source, setTimeout(step, FEED_PAUSE))
      else timers.delete(source)
    }
    timers.set(source, setTimeout(step, FEED_PAUSE))
  }
}
