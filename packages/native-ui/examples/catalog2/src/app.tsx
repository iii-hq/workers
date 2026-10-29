/**
 * The catalogue 2 example: one page per piece the kanban app needs
 * (Markdown, Lanes, drag and drop, ListDetail.listFill, named hues), each
 * with stable testTags for a host's instrumented tests (README.md lists
 * them). It calls no engine function, so a host loads it without a worker.
 */
import {
  Badge,
  Button,
  batch,
  Card,
  Chip,
  Column,
  createList,
  createSignal,
  defineApp,
  type Hue,
  Icon,
  iii,
  Lanes,
  LIMITS,
  List,
  ListDetail,
  ListRow,
  type ListSource,
  Markdown,
  type NodeRef,
  Page,
  Row,
  Scroll,
  StatusDot,
  Text,
} from '@iii-dev/native-ui'

const HUES: Hue[] = ['none', 'blue', 'purple', 'teal', 'green', 'amber', 'rose']

// ── Root ────────────────────────────────────────────────────────────────────

const PAGES = [
  ['markdown', 'Markdown'],
  ['markdownLong', 'Long Markdown'],
  ['lanes', 'Lanes'],
  ['dnd', 'Drag and drop'],
  ['fill', 'List fill'],
  ['hue', 'Hues'],
] as const

function Root() {
  return (
    <Page title="Catalogue 2" subtitle={iii.env.windowClass === 'wide' ? 'Wide window' : 'Compact window'}>
      <Column gap={8} padding={16}>
        {PAGES.map(([page, label]) => (
          <Button testTag={`open-${page}`} onPress={() => void iii.nav.push(page)}>
            {label}
          </Button>
        ))}
      </Column>
    </Page>
  )
}

// ── Markdown ────────────────────────────────────────────────────────────────

const DOC = `# Heading one

## Heading two

### Heading three

Some **bold**, some *italic*, some ~~struck~~ and some \`inline code\`.

- First item
- Second item
  - A nested item

1. One
2. Two

\`\`\`ts
const answer = 42
console.log(answer)
\`\`\`

| Key | Status | Hue |
| --- | --- | --- |
| KAN-1 | Todo | blue |
| KAN-12 | Doing | teal |

A web link: [Example](https://example.com). An app link: [KAN-12](kan:KAN-12). A bare one: https://iii.dev

> A quote.

![An image](https://example.com/image.png)

<b>raw html</b>`

function MarkdownPage() {
  const [last, setLast] = createSignal('none')
  return (
    <Page title="Markdown">
      <Scroll weight={1}>
        <Column gap={16} padding={16}>
          <Markdown testTag="md-doc" selectable onLink={setLast}>
            {DOC}
          </Markdown>
          <Text testTag="md-last-link">{last()}</Text>
          <Text role="micro" tone="muted">
            Dense
          </Text>
          <Markdown testTag="md-dense" dense onLink={setLast}>
            {'A comment with [a link](https://example.com/dense) and `code`.'}
          </Markdown>
          <Text role="micro" tone="muted">
            No link subscription
          </Text>
          <Markdown testTag="md-nolink">{'A tap on [this link](https://example.com/nothing) does nothing.'}</Markdown>
        </Column>
      </Scroll>
    </Page>
  )
}

/** Past limits.maxMarkdownChars: the host draws the start and a note. A link after the cut is never drawn. */
function longText(): string {
  const parts = [
    '# Long text\n\n',
    `This text is longer than limits.maxMarkdownChars (${LIMITS.maxMarkdownChars}). An [early link](https://example.com/early).\n\n`,
  ]
  let length = parts.join('').length
  for (let i = 1; length < LIMITS.maxMarkdownChars + 10_000; i++) {
    const p = `Paragraph ${i}: the quick brown fox jumps over the lazy dog, again and again.\n\n`
    parts.push(p)
    length += p.length
  }
  parts.push('[A link past the cut](https://example.com/past)\n')
  return parts.join('')
}

function MarkdownLongPage() {
  const [last, setLast] = createSignal('none')
  const text = longText()
  return (
    <Page title="Long Markdown" subtitle={`${text.length} characters`}>
      <Scroll weight={1}>
        <Column gap={16} padding={16}>
          <Text testTag="md-long-last-link">{last()}</Text>
          <Text testTag="md-long-length">{String(text.length)}</Text>
          <Markdown testTag="md-long" onLink={setLast}>
            {text}
          </Markdown>
        </Column>
      </Scroll>
    </Page>
  )
}

// ── Lanes ───────────────────────────────────────────────────────────────────

interface Item {
  key: string
  tag: string
  title: string
  note: string
}

const LANE_TITLES = ['Backlog', 'Todo', 'Doing', 'Review', 'Done']

function LanesPage() {
  const [selected, setSelected] = createSignal(0)
  const lanes = LANE_TITLES.map((title, i) =>
    createList<Item>({
      key: 'key',
      items: Array.from({ length: 30 }, (_, j) => ({
        key: `${i}-${j}`,
        tag: `lanes-item-${i}-${j}`,
        title: `${title} ${j + 1}`,
        note: `Lane ${i}, item ${j}`,
      })),
    }),
  )
  return (
    <Page title="Lanes">
      <Column weight={1} gap={0}>
        <Text testTag="lanes-selected" role="small" tone="muted" padding={[4, 16]}>
          {String(selected())}
        </Text>
        <Lanes
          testTag="lanes"
          weight={1}
          titles={LANE_TITLES}
          counts={lanes.map((l) => String(l.items().length))}
          selected={selected()}
          onSelect={setSelected}
        >
          {lanes.map((source, i) => (
            <List
              testTag={`lanes-list-${i}`}
              source={source}
              dividers
              templates={[(row) => <ListRow testTag={row.tag} title={row.title} subtitle={row.note} />]}
            />
          ))}
        </Lanes>
      </Column>
    </Page>
  )
}

// ── Drag and drop ───────────────────────────────────────────────────────────

interface Ticket {
  key: string
  tag: string
  title: string
  hue: Hue
  body: string
}

const ticket = (n: number, prefix: string): Ticket => ({
  key: `c${n}`,
  tag: `${prefix}-card-c${n}`,
  title: `Card ${n}`,
  hue: HUES[n % HUES.length],
  body: `## Card ${n}\n\nWhat card ${n} is about, with [a link](https://example.com/c${n}) and [KAN-${n}](kan:KAN-${n}).`,
})

const BOARD_TITLES = ['Todo', 'Doing', 'Done']

/** Three lanes of tickets: Todo c1–c5, Doing c6–c8, Done c9–c10. */
function board(prefix: string): ListSource<Ticket>[] {
  const lanes = [
    [1, 2, 3, 4, 5],
    [6, 7, 8],
    [9, 10],
  ]
  return lanes.map((ns) => createList<Ticket>({ key: 'key', items: ns.map((n) => ticket(n, prefix)) }))
}

/**
 * A drop handler as a real app writes it: find the source List by its ref,
 * take the item out of it and put it in the target at `index`, in one batch
 * (the host shows the move at once), then call the worker.
 */
function dropInto(lanes: ListSource<Ticket>[], refs: (NodeRef | undefined)[], to: number, done: (s: string) => void) {
  return (key: string | number, index: number, source: NodeRef | null) => {
    const from = source ? refs.indexOf(source) : -1
    const item = from < 0 ? undefined : lanes[from].get(key)
    if (!item) return
    batch(() => {
      lanes[from].remove(key)
      lanes[to].splice(index, 0, item)
      done(`${key}:${index}:${from}`)
    })
  }
}

interface Step {
  key: string
  tag: string
  title: string
}

function DndPage() {
  const lanes = board('dnd')
  const refs: (NodeRef | undefined)[] = []
  const [selected, setSelected] = createSignal(0)
  const [last, setLast] = createSignal('none')
  const [pressed, setPressed] = createSignal('none')
  const [longPressed, setLongPressed] = createSignal('none')
  const steps = createList<Step>({
    key: 'key',
    items: [1, 2, 3, 4, 5, 6].map((n) => ({ key: `s${n}`, tag: `order-s${n}`, title: `Step ${n}` })),
  })
  const [orderLast, setOrderLast] = createSignal('none')
  return (
    <Page title="Drag and drop">
      <Column weight={1} gap={0}>
        <Column gap={2} padding={[4, 16]}>
          <Text testTag="dnd-last" role="small">
            {last()}
          </Text>
          <Text testTag="dnd-pressed" role="small">
            {pressed()}
          </Text>
          <Text testTag="dnd-longpress" role="small">
            {longPressed()}
          </Text>
        </Column>
        <Lanes
          testTag="dnd-lanes"
          weight={1}
          titles={BOARD_TITLES}
          counts={lanes.map((l) => String(l.items().length))}
          selected={selected()}
          onSelect={setSelected}
        >
          {lanes.map((source, i) => (
            <List
              testTag={`dnd-list-${i}`}
              ref={(n) => {
                refs[i] = n
              }}
              source={source}
              dragGroup="cards"
              gap={8}
              contentPadding={[8, 12]}
              onDrop={dropInto(lanes, refs, i, setLast)}
              templates={[
                (row) => (
                  <Card
                    testTag={row.tag}
                    onPress={(item: Ticket) => setPressed(item.key)}
                    onLongPress={(item: Ticket) => setLongPressed(item.key)}
                  >
                    <Row gap={8}>
                      <StatusDot hue={row.hue} />
                      <Text role="strong" weight={1} text={row.title} />
                      <Badge hue={row.hue} text={row.key} />
                    </Row>
                  </Card>
                ),
              ]}
            />
          ))}
        </Lanes>
        <Text role="micro" tone="muted" padding={[8, 16, 4, 16]}>
          Reorder
        </Text>
        <Text testTag="dnd-order" role="small" padding={[0, 16]}>
          {steps
            .items()
            .map((s) => s.key)
            .join(',')}
        </Text>
        <Text testTag="dnd-order-last" role="small" padding={[0, 16]}>
          {orderLast()}
        </Text>
        <List
          testTag="dnd-reorder"
          height={220}
          dividers
          source={steps}
          dragGroup="order"
          onDrop={(key, index) => {
            const item = steps.get(key)
            if (!item) return
            batch(() => {
              steps.remove(key)
              steps.splice(index, 0, item)
              setOrderLast(`${key}:${index}`)
            })
          }}
          templates={[(row) => <ListRow testTag={row.tag} title={row.title} icon="list" />]}
        />
      </Column>
    </Page>
  )
}

// ── ListDetail.listFill ─────────────────────────────────────────────────────

function FillPage() {
  const lanes = board('fill')
  const [selected, setSelected] = createSignal(0)
  const [open, setOpen] = createSignal<Ticket | null>(null)
  const openCard = (lane: number) => (item: Ticket) =>
    // The narrowed board shows the card's lane: set it with showDetail, in one batch.
    batch(() => {
      setSelected(lane)
      setOpen(item)
    })
  return (
    <Page title="List fill">
      <ListDetail
        testTag="fill"
        listTitle="Board"
        listFill
        showDetail={open() !== null}
        onDismiss={() => setOpen(null)}
        list={
          <Column weight={1} gap={0}>
            <Text testTag="fill-selected" role="small" tone="muted" padding={[4, 16]}>
              {String(selected())}
            </Text>
            <Lanes
              testTag="fill-lanes"
              weight={1}
              titles={BOARD_TITLES}
              counts={lanes.map((l) => String(l.items().length))}
              selected={selected()}
              onSelect={setSelected}
            >
              {lanes.map((source, i) => (
                <List
                  testTag={`fill-list-${i}`}
                  source={source}
                  gap={8}
                  contentPadding={[8, 12]}
                  templates={[
                    (row) => (
                      <Card testTag={row.tag} onPress={openCard(i)}>
                        <Row gap={8}>
                          <Icon name="circle" hue={row.hue} />
                          <Text role="strong" weight={1} text={row.title} />
                        </Row>
                      </Card>
                    ),
                  ]}
                />
              ))}
            </Lanes>
          </Column>
        }
        detail={
          <Page
            title={open()?.title ?? ''}
            actions={
              <Button testTag="fill-close" onPress={() => setOpen(null)}>
                Close
              </Button>
            }
          >
            <Column gap={12} padding={16}>
              <Text testTag="fill-detail-id" role="id">
                {open()?.key ?? ''}
              </Text>
              <Markdown testTag="fill-detail-body" onLink={(href) => iii.ui.toast(href)}>
                {open()?.body ?? ''}
              </Markdown>
            </Column>
          </Page>
        }
      />
    </Page>
  )
}

// ── Hues ────────────────────────────────────────────────────────────────────

interface Swatch {
  key: string
  tag: string
  hue: Hue
}

function HuePage() {
  const swatches = createList<Swatch>({
    key: 'key',
    items: HUES.map((hue) => ({ key: hue, tag: `hue-item-${hue}`, hue })),
  })
  return (
    <Page title="Hues">
      <Column gap={8} padding={16} weight={1}>
        {HUES.map((hue) => (
          <Row testTag={`hue-${hue}`} gap={12}>
            <Icon testTag={`hue-icon-${hue}`} name="circle" hue={hue} size={20} />
            <Badge testTag={`hue-badge-${hue}`} hue={hue}>
              {hue}
            </Badge>
            <StatusDot testTag={`hue-dot-${hue}`} hue={hue} />
            <Chip testTag={`hue-chip-${hue}`} hue={hue}>
              {hue}
            </Chip>
          </Row>
        ))}
        <Text role="micro" tone="muted">
          Bound in a template
        </Text>
        <List
          testTag="hue-list"
          weight={1}
          source={swatches}
          templates={[
            (row) => (
              <Row testTag={row.tag} gap={12} padding={[6, 0]}>
                <StatusDot hue={row.hue} />
                <Badge hue={row.hue} text={row.key} />
              </Row>
            ),
          ]}
        />
      </Column>
    </Page>
  )
}

defineApp({
  catalog: 2,
  pages: {
    root: Root,
    markdown: MarkdownPage,
    markdownLong: MarkdownLongPage,
    lanes: LanesPage,
    dnd: DndPage,
    fill: FillPage,
    hue: HuePage,
  },
})
