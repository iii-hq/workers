/**
 * createList().set on 3,000 kanban-like items with one changed, the way an
 * app replaces its items: the same objects but one (`one`), or every item a
 * fresh copy of the worker's data (`fresh`). Each run shows its time in the
 * VM (Date.now, real time in the harness too).
 */
import { createList, createSignal, defineApp, List, ListRow, Page, Text, useTrigger } from '@iii-dev/native-ui'

interface Ticket {
  key: string
  title: string
  status: string
  priority: number
  assignee: string | null
  labels: string[]
  project: { id: string; name: string; hue: string }
  updated: string
  comments: number
}

const N = 3000
const make = (i: number): Ticket => ({
  key: `KAN-${i}`,
  title: `A ticket with a title of a usual length, number ${i}`,
  status: ['todo', 'in_progress', 'in_review', 'done'][i % 4],
  priority: i % 5,
  assignee: i % 3 ? `agent-${i % 7}` : null,
  labels: ['bug', 'ui', 'android'].slice(0, i % 4),
  project: { id: `p${i % 6}`, name: `Project ${i % 6}`, hue: 'teal' },
  updated: `2026-10-${String((i % 28) + 1).padStart(2, '0')}T12:00:00.000Z`,
  comments: i % 11,
})

const tickets = createList<Ticket>({ key: 'key', items: Array.from({ length: N }, (_, i) => make(i)) })

function Root() {
  const [ms, setMs] = createSignal('')
  const time = (fn: () => void) => {
    const t0 = Date.now()
    fn()
    setMs(String(Date.now() - t0))
  }
  let round = 0
  useTrigger(
    'demo::one',
    () => ({}),
    () =>
      time(() => {
        round++
        tickets.set(tickets.items().map((t, i) => (i === 1500 ? { ...t, title: `changed ${round}` } : t)))
      }),
  )
  useTrigger(
    'demo::fresh',
    () => ({}),
    () => {
      round++
      const data = JSON.parse(JSON.stringify(tickets.items())) as Ticket[]
      data[1500] = { ...data[1500], title: `changed ${round}` }
      time(() => tickets.set(data))
    },
  )
  // The old way, for a baseline in the same VM: each pair of items by its JSON.
  useTrigger(
    'demo::json',
    () => ({}),
    () => {
      const now = tickets.items()
      const data = JSON.parse(JSON.stringify(now)) as Ticket[]
      let changed = 0
      time(() => {
        for (let i = 0; i < now.length; i++) if (JSON.stringify(now[i]) !== JSON.stringify(data[i])) changed++
      })
      if (changed) throw new Error('a copy differs')
    },
  )
  return (
    <Page title="Set">
      <Text testTag="ms">{ms()}</Text>
      <List testTag="tickets" source={tickets} templates={[(row) => <ListRow title={row.title} />]} />
    </Page>
  )
}

defineApp({ pages: { root: Root } })
