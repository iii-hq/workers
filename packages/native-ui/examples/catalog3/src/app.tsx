/**
 * The catalogue 3 example: an overview page laid out like a dashboard, then
 * one page per piece (Checkbox, Avatar and Timestamp, Card tones, Chart and
 * Stat, Gauge and Meter, Steps, CodeView, Waveform, a chat log with
 * List.anchor and a toast with an action), each with stable testTags for a
 * host's instrumented tests and screenshots (README.md lists them). It calls
 * no engine function, so a host loads it without a worker.
 */
import {
  Avatar,
  Badge,
  Button,
  Card,
  Chart,
  type ChartSeries,
  Checkbox,
  CodeView,
  Column,
  createList,
  createSignal,
  Divider,
  defineApp,
  For,
  Gauge,
  type Hue,
  iii,
  type JSX,
  LIMITS,
  List,
  Meter,
  Page,
  Row,
  Scroll,
  Stat,
  type StepItem,
  Steps,
  Text,
  Timestamp,
  type Tone,
  Waveform,
} from '@iii-dev/native-ui'
import { AVATAR_PNG } from './avatar'

const MIN = 60_000
const HOUR = 60 * MIN
const DAY = 24 * HOUR

/** A fixed moment for the date and time styles: 27 Nov 2026, 14:05 UTC. */
const FIXED = Date.UTC(2026, 10, 27, 14, 5)

const LATENCY_API = [182, 176, 190, 210, 198, 230, 260, 240, 221, 205, 199, 188, 176, 170, 168, 181, 192, 214, 236, 228]
const LATENCY_DB = [92, 88, 95, 101, 99, 120, 131, 126, 110, 104, 97, 93, 90, 88, 86, 91, 96, 108, 118, 112]
const HOURS = LATENCY_API.map((_, i) => `${String(i + 4).padStart(2, '0')}:00`)
const RUNS = [12, 18, 9, 22, 30, 26, 14]
const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

function Section(props: { title: string; children: JSX.Element }) {
  return (
    <Column gap={8}>
      <Text role="micro" tone="muted">
        {props.title}
      </Text>
      {props.children}
    </Column>
  )
}

// ── Root ────────────────────────────────────────────────────────────────────

const PAGES = [
  ['overview', 'Overview'],
  ['checkbox', 'Checkbox'],
  ['people', 'Avatar and Timestamp'],
  ['cards', 'Card tones'],
  ['charts', 'Chart and Stat'],
  ['gauges', 'Gauge and Meter'],
  ['steps', 'Steps'],
  ['code', 'CodeView'],
  ['wave', 'Waveform'],
  ['chat', 'Chat log and toast'],
] as const

function Root() {
  return (
    <Page title="Catalogue 3" subtitle={iii.env.windowClass === 'wide' ? 'Wide window' : 'Compact window'}>
      <Scroll weight={1}>
        <Column gap={8} padding={16}>
          {PAGES.map(([page, label]) => (
            <Button testTag={`open-${page}`} onPress={() => void iii.nav.push(page)}>
              {label}
            </Button>
          ))}
        </Column>
      </Scroll>
    </Page>
  )
}

// ── Overview: a dashboard made of the new pieces ────────────────────────────

const MILESTONES: StepItem[] = [
  { title: 'Contract', detail: 'catalog.json 3', state: 'done' },
  { title: 'Hosts', detail: 'Android and Apple', state: 'active', progress: 0.6 },
  { title: 'Web port', state: 'pending' },
  { title: 'Docs', state: 'pending' },
]

function Overview() {
  const wide = () => iii.env.windowClass === 'wide'
  const [picked, setPicked] = createSignal('')
  return (
    <Page title="Overview" subtitle="native-ui · catalogue 3">
      <Scroll weight={1}>
        <Column gap={16} padding={16}>
          <Row gap={12} testTag="ov-stats">
            <Stat
              testTag="ov-agents"
              weight={1}
              label="Agents"
              value="6"
              icon="bot"
              hue="purple"
              detail="4 working · 2 waiting"
            />
            <Stat
              testTag="ov-prs"
              weight={1}
              label="Open PRs"
              value="3"
              icon="git-pull-request"
              hue="teal"
              delta="+1"
              deltaTone="success"
            />
            <Stat
              testTag="ov-cost"
              weight={1}
              label="Cost today"
              value="$4.20"
              icon="trending-up"
              hue="amber"
              series={[0.4, 0.9, 1.2, 1.8, 2.1, 2.9, 3.4, 4.2]}
              visible={wide()}
            />
          </Row>
          <Card testTag="ov-question" tone="attention" gap={10}>
            <Row gap={10} align="center">
              <Avatar name="Claude" icon="bot" hue="purple" status="warn" />
              <Column weight={1} gap={2}>
                <Text role="strong">claude · feat/catalog-3</Text>
                <Text tone="secondary">Should Chart bars of several series stack, or stand side by side?</Text>
              </Column>
              <Timestamp at={Date.now() - 4 * MIN} style="short" tone="muted" role="small" />
            </Row>
            <Row gap={8}>
              <Button kind="primary" onPress={() => setPicked('side by side')}>
                Side by side
              </Button>
              <Button kind="outline" onPress={() => setPicked('stacked')}>
                Stacked
              </Button>
            </Row>
            <Text testTag="ov-answer" tone="muted" visible={picked() !== ''}>
              {`Answered: ${picked()}`}
            </Text>
          </Card>
          <Section title="LATENCY, LAST 20 HOURS">
            <Chart
              testTag="ov-latency"
              kind="area"
              height={160}
              series={[LATENCY_API, LATENCY_DB]}
              hues={['blue', 'teal']}
              legend={['api', 'db']}
              labels={HOURS}
              thresholds={[{ value: 250, tone: 'danger', label: 'SLO' }]}
              axes
              unit="ms"
            />
          </Section>
          <Row gap={16} align="center">
            <Gauge testTag="ov-cpu" value={0.42} label="CPU" />
            <Gauge testTag="ov-mem" value={0.81} label="Memory" thresholds={[0.8, 0.95]} />
            <Gauge testTag="ov-budget" kind="half" value={0.64} text="$64" label="of $100" hue="amber" size={96} />
          </Row>
          <Section title="DISK">
            <Meter testTag="ov-disk" values={[38, 21, 12]} labels={['Worktrees', 'Logs', 'Cache']} max={100} legend />
          </Section>
          <Section title="MILESTONES">
            <Steps testTag="ov-steps" axis="horizontal" items={MILESTONES} />
          </Section>
        </Column>
      </Scroll>
    </Page>
  )
}

// ── Checkbox ────────────────────────────────────────────────────────────────

const TODOS = ['Write the contract', 'Android host', 'Apple host', 'Web port']

function CheckboxPage() {
  const [done, setDone] = createSignal<boolean[]>([true, true, false, false])
  const count = () => done().filter(Boolean).length
  const setAll = (checked: boolean) => setDone(TODOS.map(() => checked))
  return (
    <Page title="Checkbox">
      <Scroll weight={1}>
        <Column gap={12} padding={16}>
          <Checkbox
            testTag="cb-all"
            text="All"
            checked={count() === TODOS.length}
            indeterminate={count() > 0 && count() < TODOS.length}
            onChange={setAll}
          />
          <Divider />
          <For each={TODOS}>
            {(title, i) => (
              <Checkbox
                testTag={`cb-${i()}`}
                checked={done()[i()]}
                strike
                onChange={(checked) => setDone(done().map((d, j) => (j === i() ? checked : d)))}
              >
                {title}
              </Checkbox>
            )}
          </For>
          <Text testTag="cb-count" tone="muted">{`${count()} of ${TODOS.length} done`}</Text>
          <Divider />
          <Checkbox testTag="cb-disabled" checked enabled={false}>
            Disabled and checked
          </Checkbox>
          <Checkbox testTag="cb-stubborn" checked={false} onChange={() => {}}>
            The app keeps this one unchecked
          </Checkbox>
        </Column>
      </Scroll>
    </Page>
  )
}

// ── Avatar and Timestamp ────────────────────────────────────────────────────

const NAMES = ['Ada Lovelace', 'Grace Hopper', 'Linus', 'Margaret Hamilton', 'Alan Turing', 'Barbara Liskov']

function People() {
  const now = Date.now()
  return (
    <Page title="Avatar and Timestamp">
      <Scroll weight={1}>
        <Column gap={16} padding={16}>
          <Section title="INITIALS, COLOR FROM THE NAME">
            <Row gap={8}>
              <For each={NAMES}>{(name, i) => <Avatar testTag={`av-name-${i()}`} name={name} />}</For>
            </Row>
          </Section>
          <Section title="IMAGE, ICON, HUE, SHAPE">
            <Row gap={8} align="center">
              <Avatar testTag="av-image" name="Photo" image={AVATAR_PNG} size={40} />
              <Avatar testTag="av-icon" name="Claude" icon="bot" hue="purple" size={40} />
              <Avatar testTag="av-rounded" name="Codex" icon="square-terminal" hue="green" shape="rounded" size={40} />
              <Avatar testTag="av-empty" size={40} />
            </Row>
          </Section>
          <Section title="SIZES AND STATUS">
            <Row gap={12} align="center">
              <Avatar testTag="av-s20" name="Ada Lovelace" size={20} />
              <Avatar testTag="av-ok" name="Grace Hopper" size={32} status="ok" />
              <Avatar testTag="av-warn" name="Linus" size={44} status="warn" />
              <Avatar testTag="av-alert" name="Alan Turing" size={56} status="alert" />
            </Row>
          </Section>
          <Divider />
          <Section title="TIMESTAMP STYLES">
            <TimeRow label="relative, 5 min ago" tag="ts-relative" at={now - 5 * MIN} style="relative" />
            <TimeRow label="relative, 3 days ago" tag="ts-relative-days" at={now - 3 * DAY} style="relative" />
            <TimeRow label="relative, in 2 hours" tag="ts-future" at={now + 2 * HOUR} style="relative" />
            <TimeRow label="short, 3 hours ago" tag="ts-short" at={now - 3 * HOUR} style="short" />
            <TimeRow label="elapsed, running" tag="ts-elapsed" at={now - 65_000} style="elapsed" />
            <TimeRow label="date" tag="ts-date" at={FIXED} style="date" />
            <TimeRow label="time" tag="ts-time" at={FIXED} style="time" />
            <TimeRow label="dateTime" tag="ts-datetime" at={FIXED} style="dateTime" />
          </Section>
        </Column>
      </Scroll>
    </Page>
  )
}

function TimeRow(props: {
  label: string
  tag: string
  at: number
  style: 'relative' | 'short' | 'elapsed' | 'date' | 'time' | 'dateTime'
}) {
  return (
    <Row gap={8} align="center">
      <Text weight={1} tone="secondary">
        {props.label}
      </Text>
      <Timestamp testTag={props.tag} at={props.at} style={props.style} role="code" />
    </Row>
  )
}

// ── Card tones ──────────────────────────────────────────────────────────────

const TONES: Tone[] = ['default', 'attention', 'success', 'warning', 'danger', 'accent', 'muted']

function Cards() {
  return (
    <Page title="Card tones">
      <Scroll weight={1}>
        <Column gap={10} padding={16}>
          <For each={TONES}>
            {(tone) => (
              <Card testTag={`card-${tone}`} tone={tone}>
                <Text role="strong">{tone}</Text>
                <Text tone="secondary">The border and a faint fill in this tone.</Text>
              </Card>
            )}
          </For>
        </Column>
      </Scroll>
    </Page>
  )
}

// ── Chart and Stat ──────────────────────────────────────────────────────────

function Charts() {
  const [picked, setPicked] = createSignal('none')
  return (
    <Page title="Chart and Stat">
      <Scroll weight={1}>
        <Column gap={16} padding={16}>
          <Text testTag="chart-picked">{picked()}</Text>
          <Section title="LINE, TWO SERIES, AXES, THRESHOLD">
            <Chart
              testTag="chart-line"
              height={160}
              series={[LATENCY_API, LATENCY_DB]}
              legend={['api', 'db']}
              labels={HOURS}
              thresholds={[{ value: 250, tone: 'danger', label: 'SLO' }]}
              axes
              unit="ms"
              onSelect={(i) => setPicked(`line ${i} ${HOURS[i]}`)}
            />
          </Section>
          <Section title="BARS">
            <Chart
              testTag="chart-bar"
              kind="bar"
              height={120}
              series={RUNS}
              hues={['purple']}
              labels={DAYS}
              axes
              onSelect={(i) => setPicked(`bar ${i} ${DAYS[i]} ${RUNS[i]}`)}
            />
          </Section>
          <Section title="BARS, TWO SERIES SIDE BY SIDE">
            <Chart
              testTag="chart-bars2"
              kind="bar"
              height={120}
              series={[RUNS, RUNS.map((r) => Math.round(r * 0.6))]}
              hues={['blue', 'rose']}
              legend={['runs', 'failures']}
              labels={DAYS}
              axes
            />
          </Section>
          <Section title="AREA WITH A GAP, NO AXES">
            <Chart
              testTag="chart-gap"
              kind="area"
              height={64}
              series={[3, 4, 6, 5, null, null, 7, 8, 6, 9] as ChartSeries}
              hues={['green']}
            />
          </Section>
          <Divider />
          <Row gap={12}>
            <Stat
              testTag="stat-p95"
              weight={1}
              label="p95 latency"
              value="182"
              unit="ms"
              icon="timer"
              hue="blue"
              delta="-12%"
              deltaTone="success"
              series={LATENCY_API}
              onSelect={(i) => setPicked(`stat ${i} ${LATENCY_API[i]}`)}
            />
            <Stat
              testTag="stat-errors"
              weight={1}
              label="Errors"
              value="27"
              icon="triangle-alert"
              hue="rose"
              delta="+9"
              deltaTone="danger"
              series={[2, 3, 1, 4, 8, 6, 3]}
              seriesKind="bar"
            />
          </Row>
          <Stat testTag="stat-plain" label="Workers" value="14" detail="2 updated today" icon="boxes" />
        </Column>
      </Scroll>
    </Page>
  )
}

// ── Gauge and Meter ─────────────────────────────────────────────────────────

const GAUGES: [string, number][] = [
  ['ok', 0.35],
  ['warn', 0.85],
  ['danger', 0.97],
]

function Gauges() {
  const [value, setValue] = createSignal(0.5)
  return (
    <Page title="Gauge and Meter">
      <Scroll weight={1}>
        <Column gap={16} padding={16}>
          <Section title="RING, THRESHOLDS 0.8 AND 0.95">
            <Row gap={16}>
              <For each={GAUGES}>
                {([name, v]) => <Gauge testTag={`gauge-${name}`} value={v} thresholds={[0.8, 0.95]} label={name} />}
              </For>
            </Row>
          </Section>
          <Section title="HALF, HUES, TEXT">
            <Row gap={16} align="end">
              <Gauge testTag="gauge-half" kind="half" value={value()} size={120} hue="teal" label="Budget" />
              <Gauge testTag="gauge-text" value={0.66} text="2/3" hue="purple" size={80} />
            </Row>
            <Row gap={8}>
              <Button testTag="gauge-less" kind="outline" onPress={() => setValue(Math.max(0, value() - 0.1))}>
                −10%
              </Button>
              <Button testTag="gauge-more" kind="outline" onPress={() => setValue(Math.min(1, value() + 0.1))}>
                +10%
              </Button>
            </Row>
          </Section>
          <Divider />
          <Section title="METER WITH A LEGEND, OUT OF 100">
            <Meter
              testTag="meter-disk"
              values={[38, 21, 12]}
              labels={['Worktrees', 'Logs', 'Cache']}
              max={100}
              legend
            />
          </Section>
          <Section title="METER, FULL, HUES, THICK">
            <Meter
              testTag="meter-split"
              values={[5, 3, 2]}
              hues={['green', 'amber', 'rose']}
              labels={['Passed', 'Flaky', 'Failed']}
              thickness={14}
              legend
            />
          </Section>
        </Column>
      </Scroll>
    </Page>
  )
}

// ── Steps ───────────────────────────────────────────────────────────────────

const ALL_STATES: StepItem[] = [
  { title: 'Plan', detail: 'Agreed on Nov 27', state: 'done' },
  { title: 'Contract', detail: 'catalog.json 3', state: 'done' },
  { title: 'Android host', detail: 'Compose', state: 'active', progress: 0.7 },
  { title: 'Apple host', detail: 'Waiting on a question', state: 'blocked' },
  { title: 'Release 0.4', detail: 'The build broke', state: 'failed' },
  { title: 'Telemetry', state: 'skipped' },
  { title: 'Docs', state: 'pending' },
]

function StepsPage() {
  const [picked, setPicked] = createSignal('none')
  return (
    <Page title="Steps">
      <Scroll weight={1}>
        <Column gap={16} padding={16}>
          <Text testTag="steps-picked">{picked()}</Text>
          <Section title="VERTICAL, EVERY STATE">
            <Steps
              testTag="steps-vertical"
              items={ALL_STATES}
              onSelect={(i) => setPicked(`${i} ${ALL_STATES[i].title}`)}
            />
          </Section>
          <Section title="HORIZONTAL">
            <Steps testTag="steps-horizontal" axis="horizontal" items={MILESTONES} />
          </Section>
          <Section title="HORIZONTAL, COMPACT">
            <Steps testTag="steps-compact" axis="horizontal" compact items={ALL_STATES} />
          </Section>
        </Column>
      </Scroll>
    </Page>
  )
}

// ── CodeView ────────────────────────────────────────────────────────────────

const TS = `import { defineApp, Text } from '@iii-dev/native-ui'

/** The root page: one line of text. */
export function Root() {
  const name = 'catalogue 3'
  return <Text role="title">Hello, {name}!</Text>
}

defineApp({ catalog: 3, pages: { root: Root } })`

const DIFF = `@@ -12,6 +12,8 @@ export function Chart(props: ChartProps) {
   const series = normalize(props.series)
-  const max = Math.max(...series.flat())
+  const values = series.flat().filter((v) => v != null)
+  const max = props.max ?? Math.max(...values)
   return draw(series, max)
 }`

const RUST = `fn main() {
    let names = ["ada", "grace", "linus"];
    for (i, name) in names.iter().enumerate() {
        println!("{i}: {name} — a line long enough to need a sideways scroll on a phone");
    }
}`

function longCode(): string {
  const lines = []
  let length = 0
  for (let i = 1; length < LIMITS.maxCodeChars + 1000; i++) {
    const line = `const line${i} = ${i} // past limits.maxCodeChars the host draws the start and a note`
    lines.push(line)
    length += line.length + 1
  }
  return lines.join('\n')
}

function Code() {
  const [picked, setPicked] = createSignal('none')
  return (
    <Page title="CodeView">
      <Scroll weight={1}>
        <Column gap={16} padding={16}>
          <Text testTag="code-picked">{picked()}</Text>
          <Section title="TSX, LINE NUMBERS FROM 40, 43 TO 45 MARKED">
            <CodeView
              testTag="code-ts"
              language="tsx"
              lineNumbers
              startLine={40}
              highlight={[[43, 45]]}
              onSelect={(line) => setPicked(`line ${line}`)}
            >
              {TS}
            </CodeView>
          </Section>
          <Section title="DIFF">
            <CodeView testTag="code-diff" language="diff">
              {DIFF}
            </CodeView>
          </Section>
          <Section title="RUST, NO WRAP">
            <CodeView testTag="code-rust" language="rust" lineNumbers>
              {RUST}
            </CodeView>
          </Section>
          <Section title="RUST, WRAP, SELECTABLE">
            <CodeView testTag="code-wrap" language="rust" wrap selectable>
              {RUST}
            </CodeView>
          </Section>
          <Section title="PLAIN, LONGER THAN THE LIMIT">
            <CodeView testTag="code-long" height={160}>
              {longCode()}
            </CodeView>
          </Section>
        </Column>
      </Scroll>
    </Page>
  )
}

// ── Waveform ────────────────────────────────────────────────────────────────

/** A deterministic loudness envelope: two phrases and a pause. */
function peaks(n: number): number[] {
  const out: number[] = []
  for (let i = 0; i < n; i++) {
    const t = i / n
    const phrase = t < 0.42 || t > 0.55 ? 1 : 0.08
    const wobble = 0.55 + 0.45 * Math.abs(Math.sin(i * 0.37) * Math.cos(i * 0.11))
    out.push(Math.round(phrase * wobble * 100) / 100)
  }
  return out
}

const VOICE = peaks(240)
const HUES: Hue[] = ['blue', 'purple', 'teal', 'green', 'amber', 'rose']

function Wave() {
  const [progress, setProgress] = createSignal(0.3)
  return (
    <Page title="Waveform">
      <Scroll weight={1}>
        <Column gap={16} padding={16}>
          <Section title="PLAYED 30%, A MARKER AT 60%; A TAP SEEKS">
            <Waveform
              testTag="wave-voice"
              height={56}
              peaks={VOICE}
              progress={progress()}
              markers={[0.6]}
              onSelect={setProgress}
            />
            <Text testTag="wave-progress">{`${Math.round(progress() * 100)}%`}</Text>
          </Section>
          <Section title="NO PROGRESS, EACH HUE">
            <For each={HUES}>
              {(hue) => <Waveform testTag={`wave-${hue}`} height={28} peaks={peaks(80)} hue={hue} />}
            </For>
          </Section>
        </Column>
      </Scroll>
    </Page>
  )
}

// ── Chat log (List.anchor bottom) and a toast with an action ────────────────

interface Message {
  id: number
  who: string
  text: string
  at: number
}

const WHO = ['Ada Lovelace', 'Claude']
const OLDEST = 1
const FIRST = 71

function message(id: number, now: number): Message {
  return { id, who: WHO[id % 2], text: `Message ${id}`, at: now - (100 - id) * 7 * MIN }
}

function Chat() {
  const now = Date.now()
  const log = createList<Message>({ key: 'id' })
  log.set(Array.from({ length: 100 - FIRST + 1 }, (_, i) => message(FIRST + i, now)))
  const [loads, setLoads] = createSignal(0)
  const [state, setState] = createSignal('')
  const loadOlder = () => {
    const first = log.items()[0]?.id ?? OLDEST
    if (first <= OLDEST) return
    const from = Math.max(OLDEST, first - 20)
    log.splice(0, 0, ...Array.from({ length: first - from }, (_, i) => message(from + i, now)))
    setLoads(loads() + 1)
  }
  let next = 101
  const send = () => {
    log.splice(log.items().length, 0, { id: next, who: 'Claude', text: `Message ${next}`, at: Date.now() })
    next++
  }
  const deleteLast = async () => {
    const items = log.items()
    const last = items[items.length - 1]
    if (!last) return
    log.remove(last.id)
    setState(`deleted ${last.id}`)
    const undo = await iii.ui.toast(`Deleted message ${last.id}`, { action: 'Undo' })
    if (undo) {
      log.splice(log.items().length, 0, last)
      setState(`restored ${last.id}`)
    }
  }
  return (
    <Page title="Chat log" subtitle="List.anchor bottom">
      <Column weight={1}>
        <List
          testTag="chat-log"
          weight={1}
          source={log}
          anchor="bottom"
          gap={6}
          contentPadding={[12, 16]}
          onStartReached={loadOlder}
          templates={[
            (row) => (
              <Row gap={8} testTag={row.text}>
                <Avatar name={row.who} size={24} />
                <Column weight={1} gap={2}>
                  <Row gap={6} align="center">
                    <Text role="smallStrong">{row.who}</Text>
                    <Timestamp at={row.at} style="short" role="small" tone="muted" />
                  </Row>
                  <Text>{row.text}</Text>
                </Column>
              </Row>
            ),
          ]}
        />
        <Divider />
        <Row gap={8} padding={[8, 16]} align="center">
          <Badge testTag="chat-loads">{`${loads()} loads`}</Badge>
          <Text testTag="chat-state" weight={1} tone="muted">
            {state()}
          </Text>
          <Button testTag="chat-delete" kind="outline" onPress={() => void deleteLast()}>
            Delete last
          </Button>
          <Button testTag="chat-send" kind="primary" onPress={send}>
            Send
          </Button>
        </Row>
      </Column>
    </Page>
  )
}

defineApp({
  catalog: 3,
  pages: {
    root: Root,
    overview: Overview,
    checkbox: CheckboxPage,
    people: People,
    cards: Cards,
    charts: Charts,
    gauges: Gauges,
    steps: StepsPage,
    code: Code,
    wave: Wave,
    chat: Chat,
  },
})
