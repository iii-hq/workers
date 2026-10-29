/**
 * The worker UI bench app (ide: projects/worker-native-ui/design.md §11 and medicoes.md): the pages the
 * app's instrumented WorkerUiBench, TextHandshakeTest and FoldTest drive on a device, with a fake engine
 * (no worker). Every page reads its knobs from the mount params; whatever the script itself measures
 * goes to the host as `iii.call('bench::report', {...})`, which the fake engine records.
 *
 * - `root`: one line of text (the smallest first frame).
 * - `list {rows, field?}`: a templated List of `rows` rows (a ListRow with an icon, a title, a subtitle
 *   and a Badge); a press on a row counts it in the row and in the `taps` line above the list.
 *   `field: true` adds a TextField over the list (FoldTest).
 * - `prop {hz, seconds}`: one Text whose text the script changes `hz` times a second ("seq|epoch ms").
 * - `text {busyMs, everyMs, upper?}`: a TextField the script mirrors (or upper-cases), while a timer
 *   keeps the VM busy `busyMs` out of every `everyMs`.
 * - `hog {mb}`: holds `mb` MiB of typed arrays (the memory ladder).
 * - `rtt {fn, payload?, count, gapMs}`: calls `fn` `count` times in a row from inside the VM and shows the round
 *   trips in the `rtt-result` text (M9, against a real engine: the app runs this bundle as another namespace, whose
 *   functions it may call, so it reports on screen rather than through `bench::report`).
 * - `remote {session}`: a RemoteScreen on the browser worker's tab `session` with the browser app's own wire
 *   (M9 tap → frame, M14; run as the `browser` namespace against a real engine). The script only configures it:
 *   the frames go engine → native view (D5). Its `frame` events count in the `frames` text.
 */
import {
  Badge,
  Column,
  createList,
  createSignal,
  defineApp,
  iii,
  type JsonObject,
  type JsonValue,
  List,
  ListRow,
  onCleanup,
  onMount,
  Page,
  Progress,
  RemoteScreen,
  Show,
  Text,
  TextField,
} from '@iii-dev/native-ui'

type Params = { params: JsonObject }

function num(v: JsonValue | undefined, fallback: number): number {
  return typeof v === 'number' && Number.isFinite(v) ? v : fallback
}

/** One measurement of the script, to the fake engine (never awaited: a report must not slow the page). */
function report(kind: string, fields: JsonObject): void {
  void iii.call('bench::report', { kind, ...fields }).catch(() => {})
}

function Root() {
  return (
    <Page title="Bench">
      <Text testTag="hello" padding={16}>
        Worker UI bench
      </Text>
    </Page>
  )
}

interface Row {
  key: number
  tag: string
  title: string
  sub: string
  count: string
}

function ListPage(props: Params) {
  const rows = num(props.params.rows, 1000)
  const t0 = performance.now()
  const items: Row[] = new Array(rows)
  for (let i = 0; i < rows; i++) items[i] = { key: i, tag: `r${i}`, title: `Row ${i}`, sub: `item ${i}`, count: '0' }
  const source = createList<Row>({ key: 'key', items })
  const buildMs = performance.now() - t0
  const [taps, setTaps] = createSignal(0)
  const [text, setText] = createSignal('')
  onMount(() => report('list', { rows, buildMs }))
  const press = (item: Row) => {
    const n = Number(item.count) + 1
    source.update(item.key, { count: String(n), sub: `item ${item.key} · ${n} taps` })
    setTaps((t) => t + 1)
  }
  return (
    <Page title={`List ${rows}`}>
      <Column weight={1} gap={0}>
        <Text testTag="taps" role="small" tone="muted" padding={[4, 16]}>
          {`taps ${taps()}`}
        </Text>
        <Show when={props.params.field === true}>
          <TextField testTag="field" placeholder="Type here" value={text()} onChange={setText} padding={[4, 12]} />
        </Show>
        <List
          testTag="list"
          weight={1}
          dividers
          source={source}
          templates={[
            (row) => (
              <ListRow
                testTag={row.tag}
                title={row.title}
                subtitle={row.sub}
                icon="globe"
                onPress={press}
                trailing={<Badge text={row.count} />}
              />
            ),
          ]}
        />
      </Column>
    </Page>
  )
}

function PropPage(props: Params) {
  const hz = num(props.params.hz, 60)
  const seconds = num(props.params.seconds, 5)
  const period = 1000 / hz
  const [value, setValue] = createSignal('0|0')
  const [progress, setProgress] = createSignal(0)
  const late: number[] = []
  const cost: number[] = []
  let seq = 0
  let timer = 0
  let start = 0
  const tick = () => {
    const now = performance.now()
    late.push(now - (start + seq * period))
    seq++
    const t = performance.now()
    setValue(`${seq}|${Date.now()}`)
    if (seq % 10 === 0) setProgress(Math.min(1, (now - start) / (seconds * 1000)))
    cost.push(performance.now() - t)
    if (now - start >= seconds * 1000) {
      report('prop', { hz, seconds, ticks: seq, lateMs: summary(late), setMs: summary(cost) })
      return
    }
    timer = setTimeout(tick, Math.max(0, start + seq * period - performance.now()))
  }
  onMount(() => {
    start = performance.now()
    timer = setTimeout(tick, period)
  })
  onCleanup(() => clearTimeout(timer))
  return (
    <Page title={`Prop ${hz} Hz`}>
      <Column gap={12} padding={16}>
        <Text testTag="value" role="title">
          {value()}
        </Text>
        <Progress value={progress()} />
      </Column>
    </Page>
  )
}

function TextPage(props: Params) {
  const busyMs = num(props.params.busyMs, 200)
  const everyMs = num(props.params.everyMs, 300)
  const upper = props.params.upper === true
  const [text, setText] = createSignal('')
  const [edits, setEdits] = createSignal(0)
  let busyTurns = 0
  if (busyMs > 0) {
    const id = setInterval(() => {
      const t = performance.now()
      while (performance.now() - t < busyMs) {
        // busy: the VM's thread is taken, its entries wait
      }
      busyTurns++
    }, everyMs)
    onCleanup(() => clearInterval(id))
  }
  const change = (t: string) => {
    setEdits((e) => e + 1)
    // upper: a write the host applies only when it saw every edit (the handshake), else drops.
    setText(upper ? t.toUpperCase() : t)
  }
  onCleanup(() => report('text', { busyMs, everyMs, upper, edits: edits(), busyTurns }))
  return (
    <Page title="Text">
      <Column gap={8} padding={16}>
        <TextField
          testTag="field"
          placeholder="Type here"
          value={text()}
          onChange={change}
          autofocus
          autocorrect={false}
        />
        <Text testTag="edits" role="small" tone="muted">
          {`edits ${edits()}`}
        </Text>
        <Text testTag="echo">{text()}</Text>
      </Column>
    </Page>
  )
}

function HogPage(props: Params) {
  const mb = num(props.params.mb, 16)
  // Typed arrays are allocated through the VM's allocator (they count against its heap cap); filled so the
  // pages are really touched.
  const held: Uint8Array[] = []
  for (let i = 0; i < mb; i++) held.push(new Uint8Array(1 << 20).fill(i & 255))
  onMount(() => report('hog', { mb, held: held.length }))
  return (
    <Page title="Hog">
      <Text testTag="hog" padding={16}>
        {`holding ${held.length} MiB`}
      </Text>
    </Page>
  )
}

function RttPage(props: Params) {
  const fn = typeof props.params.fn === 'string' ? props.params.fn : 'browser::chromium::status'
  const count = num(props.params.count, 50)
  const gapMs = num(props.params.gapMs, 100)
  const payload = props.params.payload ?? {}
  const [done, setDone] = createSignal(0)
  const [result, setResult] = createSignal('')
  onMount(async () => {
    const rtt: number[] = []
    let failed = 0
    for (let i = 0; i < count; i++) {
      const t = performance.now()
      try {
        await iii.call(fn, payload)
        rtt.push(performance.now() - t)
      } catch {
        failed++
      }
      setDone(i + 1)
      await new Promise((resolve) => setTimeout(resolve, gapMs))
    }
    setResult(JSON.stringify({ fn, count, failed, rttMs: summary(rtt) }))
  })
  return (
    <Page title="Round trip">
      <Column gap={8} padding={16}>
        <Text testTag="rtt-done">{`${done()} / ${count}`}</Text>
        <Text testTag="rtt-result" role="code">
          {result()}
        </Text>
      </Column>
    </Page>
  )
}

/** The browser app's wire (workers browser/native-ui/src/api.ts `WIRE`), so the view behaves as the app's. */
const WIRE: JsonObject = {
  frames: { trigger: 'browser::frame-event', config: { session_id: '$session' } },
  seed: { fn: 'browser::frame', args: { session_id: '$session', since_frame: '$seq' } },
  start: {
    fn: 'browser::screencast::start',
    args: { session_id: '$session', viewer_id: '$viewer', lease_ms: 20000, scale: '$scale' },
    renewMs: 6000,
  },
  stop: { fn: 'browser::screencast::stop', args: { session_id: '$session', viewer_id: '$viewer' } },
  resize: {
    fn: 'browser::resize',
    args: {
      session_id: '$session',
      width: '$w',
      height: '$h',
      device_scale_factor: '$dpr',
      mobile: '$mobile',
      fit: true,
    },
    debounceMs: 180,
    minChangeDp: 4,
  },
  input: {
    fn: 'browser::act',
    tap: { session_id: '$session', action: 'click', x: '$x', y: '$y', button: 'left', click_count: '$count' },
    scroll: { session_id: '$session', action: 'scroll', x: '$x', y: '$y', delta_x: '$dx', delta_y: '$dy' },
    type: { session_id: '$session', action: 'type', text: '$text' },
    key: { session_id: '$session', action: 'press', key: '$key' },
  },
}

function RemotePage(props: Params) {
  const session = typeof props.params.session === 'string' ? props.params.session : ''
  const [frames, setFrames] = createSignal(0)
  const [error, setError] = createSignal('')
  return (
    <Page title="Remote">
      <Column weight={1} gap={0}>
        <Text testTag="frames" role="small" tone="muted" padding={[4, 16]}>
          {`frame events ${frames()} ${error()}`}
        </Text>
        <RemoteScreen
          testTag="screen"
          weight={1}
          session={session}
          wire={WIRE}
          mobile
          mode="interact"
          enabled
          onFrame={() => setFrames((n) => n + 1)}
          onError={(message: string) => setError(message)}
        />
      </Column>
    </Page>
  )
}

/** P50/P90/P99 and the max of a sample, in its unit. */
function summary(xs: number[]): JsonObject {
  if (!xs.length) return { n: 0 }
  const s = [...xs].sort((a, b) => a - b)
  const at = (q: number) => s[Math.min(s.length - 1, Math.floor(q * s.length))]
  return { n: s.length, p50: at(0.5), p90: at(0.9), p99: at(0.99), max: s[s.length - 1] }
}

defineApp({
  pages: {
    root: Root,
    list: ({ params }) => <ListPage params={params} />,
    prop: ({ params }) => <PropPage params={params} />,
    text: ({ params }) => <TextPage params={params} />,
    hog: ({ params }) => <HogPage params={params} />,
    rtt: ({ params }) => <RttPage params={params} />,
    remote: ({ params }) => <RemotePage params={params} />,
  },
})
