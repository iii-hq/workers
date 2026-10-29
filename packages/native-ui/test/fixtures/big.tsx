/** A list whose items the test sizes, to pass limits.maxMessageBytes. */
import { createList, defineApp, List, ListRow, Page, useTrigger } from '@iii-dev/native-ui'

const rows = createList<{ key: string; title: string }>({ key: 'key' })

function Root() {
  useTrigger<{ count: number; size: number }>(
    'demo::fill',
    () => ({}),
    ({ count, size }) => {
      for (let i = 0; i < count; i++)
        rows.splice(rows.items().length, 0, { key: `k${rows.items().length}`, title: 'x'.repeat(size) })
    },
  )
  return (
    <Page title="Big">
      <List source={rows} templates={[(row) => <ListRow title={row.title} />]} />
    </Page>
  )
}

defineApp({ pages: { root: Root } })
