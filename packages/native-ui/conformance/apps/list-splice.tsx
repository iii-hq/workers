/**
 * A List with two templates chosen by `kind` (rows and section headers), a
 * header, an empty state and a footer; a trigger replaces its items, and
 * createList().set sends only the splices that changed.
 */
import {
  createList,
  defineApp,
  EmptyState,
  IconButton,
  List,
  ListRow,
  Page,
  Text,
  useTrigger,
} from '@iii-dev/native-ui'

interface Row {
  key: string
  kind: 0 | 1
  title: string
  url?: string
}

const rows = createList<Row>({ key: 'key', kind: 'kind' })

function Tabs() {
  useTrigger<{ rows: Row[] }>(
    'demo::tabs-changed',
    () => ({}),
    (payload) => rows.set(payload.rows),
  )
  return (
    <Page title="Tabs">
      <List
        source={rows}
        testTag="tabs"
        dividers
        header={<Text role="micro">All tabs</Text>}
        templates={[
          (row) => (
            <ListRow
              title={row.title}
              subtitle={row.url}
              icon="globe"
              onPress={(item: Row) => rows.update(item.key, { title: `${item.title} (opened)` })}
              trailing={
                <IconButton icon="x" label="Close tab" testTag="close" onPress={(item: Row) => rows.remove(item.key)} />
              }
            />
          ),
          (row) => <Text role="micro" text={row.title} />,
        ]}
        empty={<EmptyState icon="globe" title="No tabs" />}
        footer={<Text tone="faint">End of the list</Text>}
      />
    </Page>
  )
}

defineApp({ pages: { root: Tabs } })
