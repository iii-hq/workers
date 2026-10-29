/**
 * The hello app: the smallest app that touches every part of the host a
 * worker app needs (a page, a templated list, a text field, buttons, a
 * toast, an action menu, storage, env). It calls no engine function, so a
 * host can load it without a worker behind it.
 */
import {
  Button,
  Column,
  createList,
  createSignal,
  defineApp,
  EmptyState,
  IconButton,
  iii,
  List,
  ListRow,
  onMount,
  Page,
  Row,
  Text,
  TextField,
  useEnv,
} from '@iii-dev/native-ui'

interface Greeting {
  key: string
  name: string
  note: string
}

const greetings = createList<Greeting>({ key: 'key' })
let next = 1

function add(name: string) {
  const trimmed = name.trim()
  if (!trimmed) return false
  greetings.splice(0, 0, { key: `g${next++}`, name: `Hello, ${trimmed}!`, note: `#${greetings.items().length + 1}` })
  void iii.storage
    .set(
      'names',
      greetings.items().map((g) => g.name),
    )
    .catch(() => {})
  return true
}

function Hello() {
  const env = useEnv()
  const [name, setName] = createSignal('')
  const submit = () => {
    if (add(name())) setName('')
    else iii.ui.toast('Type a name first', 'error')
  }
  onMount(() => {
    iii.storage
      .get<string[]>('names')
      .then((names) => {
        for (const n of [...(names ?? [])].reverse()) {
          greetings.splice(0, 0, { key: `g${next++}`, name: n, note: 'saved' })
        }
      })
      .catch(() => {})
  })
  return (
    <Page
      title="Hello"
      subtitle={env.windowClass === 'wide' ? 'Wide window' : 'Compact window'}
      actions={<IconButton testTag="add-world" icon="plus" label="Greet the world" onPress={() => add('world')} />}
      bottom={
        <Row gap={8} padding={[8, 12]}>
          <TextField
            testTag="name"
            weight={1}
            placeholder="A name"
            value={name()}
            onChange={setName}
            onSubmit={submit}
            imeAction="done"
            clearable
          />
          <Button testTag="greet" kind="primary" onPress={submit}>
            Greet
          </Button>
        </Row>
      }
    >
      <Column gap={0} weight={1}>
        <Text role="micro" tone="muted" padding={[8, 16]}>
          Greetings
        </Text>
        <List
          testTag="greetings"
          weight={1}
          dividers
          source={greetings}
          templates={[
            (row) => (
              <ListRow
                title={row.name}
                subtitle={row.note}
                icon="globe"
                onPress={(item: Greeting) => iii.ui.toast(item.name)}
                onLongPress={async (item: Greeting) => {
                  const choice = await iii.ui.actionMenu(null, {
                    title: item.name,
                    actions: [
                      { label: 'Copy', icon: 'copy' },
                      { label: 'Remove', icon: 'trash', destructive: true },
                    ],
                  })
                  if (choice === 0) await iii.clipboard.writeText(item.name)
                  if (choice === 1) greetings.remove(item.key)
                }}
              />
            ),
          ]}
          empty={<EmptyState icon="globe" title="No greetings yet" description="Type a name below and tap Greet." />}
        />
      </Column>
    </Page>
  )
}

defineApp({ pages: { root: Hello } })
