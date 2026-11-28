/** Catalogue 3: the display and data types, a to-do list, a toast with an action and a chat log. */
import {
  Avatar,
  Button,
  Card,
  Chart,
  Checkbox,
  CodeView,
  Column,
  createList,
  createSignal,
  defineApp,
  Gauge,
  iii,
  List,
  Meter,
  Stat,
  Steps,
  Text,
  Timestamp,
  Waveform,
} from '@iii-dev/native-ui'

const log = createList<{ id: number; text: string }>({ key: 'id', items: [{ id: 10, text: 'latest' }] })

function Root() {
  const [done, setDone] = createSignal(false)
  const [some, setSome] = createSignal(true)
  const [last, setLast] = createSignal('')
  const [undo, setUndo] = createSignal('')
  return (
    <Column gap={8}>
      <Avatar testTag="ada" name="Ada Lovelace" status="warn" size={28} />
      <Avatar testTag="bot" icon="bot" hue="purple" shape="rounded" />
      <Timestamp testTag="when" at={1764250000000} style="elapsed" role="small" tone="muted" />
      <Checkbox testTag="todo" checked={done()} strike onChange={setDone}>
        Ship catalogue {3}
      </Checkbox>
      <Checkbox
        testTag="all"
        text="All"
        checked={false}
        indeterminate={some()}
        onChange={(c) => {
          setSome(false)
          setLast(`all ${c}`)
        }}
      />
      <Card testTag="card" tone="danger">
        <Text>Build failed</Text>
      </Card>
      <Stat testTag="stat" label="Runs" value="1.2k" series={[1, 2, 3]} onSelect={(i) => setLast(`stat ${i}`)} />
      <Chart
        testTag="chart"
        kind="bar"
        series={[3, 5, null, 2]}
        hues={['amber']}
        thresholds={[{ value: 4, tone: 'warning' }]}
        onSelect={(i) => setLast(`chart ${i}`)}
      />
      <Gauge testTag="gauge" kind="half" value={0.5} thresholds={[0.6, 0.9]} />
      <Meter testTag="meter" values={[1, 2]} hues={['teal', 'rose']} thickness={6} />
      <Steps
        testTag="steps"
        axis="horizontal"
        compact
        items={[
          { title: 'A', state: 'done' },
          { title: 'B', state: 'blocked' },
        ]}
        onSelect={(i) => setLast(`step ${i}`)}
      />
      <CodeView
        testTag="code"
        language="diff"
        startLine={5}
        highlight={[[5, 6]]}
        onSelect={(l) => setLast(`line ${l}`)}
      >
        {'+ added\n- removed'}
      </CodeView>
      <Waveform testTag="wave" peaks={[0.2, 0.8]} onSelect={(f) => setLast(`seek ${f}`)} />
      <Text testTag="last">{last()}</Text>
      <Button testTag="toast" onPress={async () => setUndo(String(await iii.ui.toast('Deleted', { action: 'Undo' })))}>
        Delete
      </Button>
      <Text testTag="undo">{undo()}</Text>
      <List
        testTag="log"
        source={log}
        anchor="bottom"
        onStartReached={() => log.splice(0, 0, { id: log.items()[0].id - 1, text: 'older' })}
        templates={[(row) => <Text>{row.text}</Text>]}
      />
    </Column>
  )
}

/** A Checkbox whose app keeps it unchecked. */
function Stubborn() {
  return <Checkbox testTag="stubborn" checked={false} onChange={() => {}} text="Never" />
}

defineApp({ catalog: 3, pages: { root: Root, stubborn: Stubborn } })
