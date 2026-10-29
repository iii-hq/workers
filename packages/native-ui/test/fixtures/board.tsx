/**
 * Catalogue 2: a ListDetail with listFill whose list is a Lanes of three
 * Lists in one drag group (cards with a bound hue), a Markdown detail with
 * links, and a Lanes that does not take the person's lane.
 */
import {
  Badge,
  batch,
  Card,
  Column,
  createList,
  createSignal,
  defineApp,
  Icon,
  itemHandler,
  Lanes,
  List,
  ListDetail,
  Markdown,
  type NodeRef,
  Page,
  Text,
} from '@iii-dev/native-ui'

interface Ticket {
  id: string
  title: string
  hue: string
  body: string
}

const ticket = (id: string, hue: string): Ticket => ({
  id,
  title: `Ticket ${id}`,
  hue,
  body: `# ${id}\n\nSee [the docs](https://example.com/${id}) and [KAN-2](kan:KAN-2).`,
})

const TITLES = ['Todo', 'Doing', 'Done']
const lanes = [
  createList<Ticket>({ key: 'id', items: [ticket('a', 'blue'), ticket('b', 'teal'), ticket('c', 'rose')] }),
  createList<Ticket>({ key: 'id', items: [ticket('d', 'amber')] }),
  createList<Ticket>({ key: 'id', items: [] }),
]

function Board() {
  const [selected, setSelected] = createSignal(0)
  const [open, setOpen] = createSignal<Ticket | null>(null)
  const [lastDrop, setLastDrop] = createSignal('')
  const [lastLink, setLastLink] = createSignal('')
  const [pressed, setPressed] = createSignal('')
  const refs: (NodeRef | undefined)[] = []

  const drop = (to: number) => (key: string | number, index: number, source: NodeRef | null) => {
    const from = source ? refs.indexOf(source) : -1
    const item = from < 0 ? undefined : lanes[from].get(key)
    setLastDrop(`${key}:${index}:${from}`)
    if (!item) return
    batch(() => {
      lanes[from].remove(key)
      lanes[to].splice(index, 0, item)
    })
  }
  const openCard = (lane: number) => (item: Ticket) => {
    // The narrowed board shows the card's lane: selected and showDetail in one batch.
    batch(() => {
      setSelected(lane)
      setOpen(item)
    })
  }

  return (
    <Page title="Board">
      <ListDetail
        testTag="ld"
        listTitle="Board"
        listFill
        showDetail={open() !== null}
        onDismiss={() => setOpen(null)}
        list={
          <Column weight={1}>
            <Text testTag="last-drop">{lastDrop()}</Text>
            <Text testTag="pressed">{pressed()}</Text>
            <Lanes
              testTag="lanes"
              titles={TITLES}
              counts={lanes.map((l) => String(l.items().length))}
              selected={selected()}
              onSelect={setSelected}
            >
              {lanes.map((source, i) => (
                <List
                  testTag={`lane-${i}`}
                  ref={(n) => {
                    refs[i] = n
                  }}
                  source={source}
                  dragGroup="cards"
                  onDrop={drop(i)}
                  templates={[
                    (row) => (
                      <Card onPress={openCard(i)} onLongPress={(item: Ticket) => setPressed(`${item.id} ${item.hue}`)}>
                        <Text text={row.title} />
                        <Badge testTag="hue" hue={row.hue} text={row.id} />
                      </Card>
                    ),
                  ]}
                />
              ))}
            </Lanes>
          </Column>
        }
        detail={
          <Page title={open()?.title ?? ''}>
            <Column>
              <Icon testTag="icon" name="circle" hue="teal" />
              <Markdown testTag="body" dense onLink={setLastLink}>
                {open()?.body ?? ''}
              </Markdown>
              <Text testTag="last-link">{lastLink()}</Text>
            </Column>
          </Page>
        }
      />
    </Page>
  )
}

interface Comment {
  key: string
  body: string
}

/** Comments: a Markdown per row, its links with the item first. */
function Comments() {
  const [last, setLast] = createSignal('')
  const comments = createList<Comment>({
    key: 'key',
    items: [
      { key: 'c1', body: 'See [KAN-2](kan:KAN-2).' },
      { key: 'c2', body: 'Done in https://example.com/pr/1' },
    ],
  })
  return (
    <Column>
      <Text testTag="comment-link">{last()}</Text>
      <List
        testTag="comments"
        source={comments}
        templates={[
          (row) => (
            <Markdown
              testTag="comment"
              dense
              text={row.body}
              onLink={itemHandler((c: Comment, href: string) => setLast(`${c.key} ${href}`))}
            />
          ),
        ]}
      />
    </Column>
  )
}

/** A Lanes whose app keeps lane 1, whatever the person picks. */
function Stubborn() {
  return (
    <Lanes testTag="stubborn" titles={['One', 'Two', 'Three']} selected={1} onSelect={() => {}}>
      <Text>one</Text>
      <Text>two</Text>
      <Text>three</Text>
    </Lanes>
  )
}

defineApp({ catalog: 2, pages: { root: Board, stubborn: Stubborn, comments: Comments } })
