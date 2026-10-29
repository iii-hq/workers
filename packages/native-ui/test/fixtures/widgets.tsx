/** RemoteScreen, ListDetail, a templated List with nested bindings, TextField writes and focus. */
import {
  Badge,
  createList,
  createSignal,
  defineApp,
  iii,
  List,
  ListDetail,
  ListRow,
  type NodeRef,
  Page,
  RemoteScreen,
  Text,
  TextField,
} from '@iii-dev/native-ui'

const WIRE = {
  frames: { trigger: 'demo::frame-event', config: { session_id: '$session' } },
  start: { fn: 'demo::screencast::start', args: { session_id: '$session', viewer_id: '$viewer' }, renewMs: 6000 },
}

interface Row {
  key: string
  meta: { host: string; secure: boolean }
  title: string
}

const rows = createList<Row>({ key: 'key' })
rows.set([{ key: 'a', meta: { host: 'example.com', secure: true }, title: 'Example' }])

function Root() {
  const [show, setShow] = createSignal(false)
  const [query, setQuery] = createSignal('')
  const [point, setPoint] = createSignal('')
  let field: NodeRef | undefined
  return (
    <Page title="Widgets">
      <ListDetail
        testTag="ld"
        listTitle="Tabs"
        showDetail={show()}
        onDismiss={() => setShow(false)}
        list={
          <List
            source={rows}
            templates={[
              (row) => (
                <ListRow
                  title={row.title}
                  subtitle={row.meta.host}
                  onPress={() => setShow(true)}
                  trailing={<Badge testTag="host">{row.meta.host}</Badge>}
                />
              ),
            ]}
          />
        }
        detail={
          <Page
            title="Detail"
            bottom={<TextField ref={field} testTag="q" value={query()} onChange={setQuery} clearable />}
          >
            <RemoteScreen
              testTag="screen"
              session="s1"
              wire={WIRE}
              mode="view"
              placeholder="Loading…"
              onLongPress={(x, y) => setPoint(`${x},${y}`)}
              onFrame={(w, h) => setPoint(`${w}x${h}`)}
            />
            <Text testTag="point">{point()}</Text>
            <Text testTag="focus" onPress={() => field && iii.ui.focus(field)}>
              Focus
            </Text>
            <Text testTag="clear" onPress={() => setQuery('')}>
              Clear
            </Text>
          </Page>
        }
      />
    </Page>
  )
}

defineApp({ pages: { root: Root } })
