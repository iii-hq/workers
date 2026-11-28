/**
 * Catalogue 3, List.anchor bottom (a chat log): the newest message is the
 * last item; startReached (event 16) loads two older ones in at index 0.
 */
import { createList, defineApp, List, Page, Text } from '@iii-dev/native-ui'

interface Message {
  id: number
  text: string
}

const log = createList<Message>({ key: 'id' })
log.set([
  { id: 3, text: 'Third' },
  { id: 4, text: 'Fourth' },
])

function loadOlder() {
  const first = log.items()[0]?.id ?? 0
  if (first <= 1) return
  log.splice(0, 0, { id: first - 2, text: `Message ${first - 2}` }, { id: first - 1, text: `Message ${first - 1}` })
}

function Root() {
  return (
    <Page title="Log">
      <List
        testTag="log"
        source={log}
        anchor="bottom"
        gap={4}
        onStartReached={loadOlder}
        templates={[(row) => <Text>{row.text}</Text>]}
      />
    </Page>
  )
}

defineApp({ catalog: 3, pages: { root: Root } })
