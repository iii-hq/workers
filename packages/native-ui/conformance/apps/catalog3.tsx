/**
 * Catalogue 3, the display and data types: an Avatar with a status and one
 * with an icon, a Timestamp, a Checkbox, a Card with a tone, a Stat with a
 * sparkline, a Chart with two series, a threshold and axes, a Gauge, a
 * Meter, Steps, a CodeView with marked lines and a Waveform. Each event
 * writes what it got in the `last` Text.
 */
import {
  Avatar,
  Card,
  Chart,
  Checkbox,
  CodeView,
  Column,
  createSignal,
  defineApp,
  Gauge,
  Meter,
  Page,
  Row,
  Stat,
  Steps,
  Text,
  Timestamp,
  Waveform,
} from '@iii-dev/native-ui'

const CODE = `export function add(a: number, b: number) {
  return a + b
}`

function Root() {
  const [done, setDone] = createSignal(false)
  const [last, setLast] = createSignal('')
  return (
    <Page title="Catalogue 3">
      <Column gap={12} padding={16}>
        <Row gap={8}>
          <Avatar testTag="ada" name="Ada Lovelace" status="ok" />
          <Avatar testTag="agent" name="Claude" icon="bot" hue="purple" shape="rounded" size={40} />
          <Timestamp testTag="when" at={1764250000000} style="short" tone="muted" />
        </Row>
        <Checkbox testTag="todo" checked={done()} strike onChange={setDone}>
          Write the tests
        </Checkbox>
        <Card testTag="question" tone="attention">
          <Text>Which branch?</Text>
        </Card>
        <Stat
          testTag="p95"
          label="p95"
          value="182"
          unit="ms"
          icon="timer"
          delta="-12%"
          deltaTone="success"
          series={[240, 220, 198, null, 190, 182]}
        />
        <Chart
          testTag="latency"
          kind="area"
          series={[
            [120, 140, 180, 160],
            [80, 90, 85, 100],
          ]}
          hues={['blue', 'rose']}
          labels={['Mon', 'Tue', 'Wed', 'Thu']}
          legend={['api', 'db']}
          thresholds={[{ value: 170, tone: 'danger', label: 'SLO' }]}
          axes
          unit="ms"
          onSelect={(i) => setLast(`chart ${i}`)}
        />
        <Row gap={12}>
          <Gauge testTag="cpu" value={0.72} thresholds={[0.8, 0.95]} label="CPU" />
          <Gauge testTag="budget" kind="half" value={0.9} text="90%" thresholds={[0.8]} hue="teal" size={96} />
        </Row>
        <Meter testTag="disk" values={[40, 25, 10]} labels={['Code', 'Logs', 'Cache']} max={100} legend />
        <Steps
          testTag="plan"
          items={[
            { title: 'Contract', state: 'done' },
            { title: 'Hosts', detail: 'Android and Apple', state: 'active', progress: 0.4 },
            { title: 'Docs', state: 'pending' },
          ]}
          onSelect={(i) => setLast(`step ${i}`)}
        />
        <CodeView
          testTag="code"
          language="ts"
          lineNumbers
          startLine={10}
          highlight={[11]}
          onSelect={(line) => setLast(`line ${line}`)}
        >
          {CODE}
        </CodeView>
        <Waveform
          testTag="voice"
          peaks={[0.1, 0.5, 0.9, 0.4, 0.2, 0.7]}
          progress={0.3}
          markers={[0.5]}
          hue="green"
          onSelect={(f) => setLast(`seek ${f}`)}
        />
        <Text testTag="last">{last()}</Text>
      </Column>
    </Page>
  )
}

defineApp({ catalog: 3, pages: { root: Root } })
