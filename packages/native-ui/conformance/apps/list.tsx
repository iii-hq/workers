/**
 * §5.7 "Uma lista": a List (keyField "id") whose template 0 is a ListRow with
 * the title and the url bound, the subtitle in mono, press and longPress.
 * A tap on a row changes that item's title.
 */
import { createList, defineApp, List, ListRow } from '@iii-dev/native-ui'

interface Tab {
  id: string
  title: string
  url: string
}

const tabs = createList<Tab>({ key: 'id' })
tabs.set([
  { id: 's1', title: 'Example Domain', url: 'example.com' },
  { id: 's2', title: 'iii', url: 'iii.dev' },
])

function Tabs() {
  return (
    <List
      source={tabs}
      templates={[
        (row) => (
          <ListRow
            title={row.title}
            subtitle={row.url}
            subtitleMono
            onPress={(item: Tab) => tabs.update(item.id, { title: 'iii — More application.' })}
            onLongPress={(item: Tab) => tabs.remove(item.id)}
          />
        ),
      ]}
    />
  )
}

defineApp({ pages: { root: Tabs } })
