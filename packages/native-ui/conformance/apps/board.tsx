/**
 * Catalogue 2, a board: a ListDetail with listFill whose list is a Lanes of
 * three Lists in the drag group "cards" (titles and counts), each card a
 * Card with its title and a Badge whose hue is bound to the item's (by name
 * in the script, by index on the wire). A drop splices the lists in its
 * handler; a tap opens the card, setting the lane and showDetail in one
 * batch.
 */
import {
  Badge,
  batch,
  Card,
  createList,
  createSignal,
  defineApp,
  Lanes,
  List,
  ListDetail,
  type NodeRef,
  Page,
  Text,
} from '@iii-dev/native-ui'

interface Ticket {
  id: string
  title: string
  hue: string
}

const lanes = [
  createList<Ticket>({
    key: 'id',
    items: [
      { id: 'KAN-1', title: 'Board', hue: 'blue' },
      { id: 'KAN-2', title: 'Lanes', hue: 'teal' },
      { id: 'KAN-3', title: 'Drops', hue: 'rose' },
    ],
  }),
  createList<Ticket>({ key: 'id', items: [{ id: 'KAN-4', title: 'Hues', hue: 'amber' }] }),
  createList<Ticket>({ key: 'id', items: [] }),
]

function Board() {
  const [lane, setLane] = createSignal(0)
  const [open, setOpen] = createSignal<Ticket | null>(null)
  const refs: (NodeRef | undefined)[] = []
  const drop = (to: number) => (key: string | number, index: number, source: NodeRef | null) => {
    const from = source ? refs.indexOf(source) : -1
    const item = from < 0 ? undefined : lanes[from].get(key)
    if (!item) return
    batch(() => {
      lanes[from].remove(key)
      lanes[to].splice(index, 0, item)
    })
  }
  const openCard = (i: number) => (item: Ticket) =>
    batch(() => {
      setLane(i)
      setOpen(item)
    })
  return (
    <ListDetail
      testTag="board"
      listTitle="Board"
      listFill
      showDetail={open() !== null}
      onDismiss={() => setOpen(null)}
      list={
        <Lanes
          testTag="lanes"
          titles={['Todo', 'Doing', 'Done']}
          counts={lanes.map((l) => String(l.items().length))}
          selected={lane()}
          onSelect={setLane}
        >
          {lanes.map((source, i) => (
            <List
              testTag={`lane-${i}`}
              ref={(n) => {
                refs[i] = n
              }}
              source={source}
              dragGroup="cards"
              gap={8}
              onDrop={drop(i)}
              templates={[
                (row) => (
                  <Card onPress={openCard(i)}>
                    <Text role="strong" text={row.title} />
                    <Badge hue={row.hue} text={row.id} />
                  </Card>
                ),
              ]}
            />
          ))}
        </Lanes>
      }
      detail={
        <Page title={open()?.title ?? ''}>
          <Text testTag="ticket">{open()?.id ?? ''}</Text>
        </Page>
      }
    />
  )
}

defineApp({ catalog: 2, pages: { root: Board } })
