/** §5.7 "Ambiente": the page shows the window class and follows env changes. */
import { defineApp, Page, Text, useEnv } from '@iii-dev/native-ui'

function Root() {
  const env = useEnv()
  return (
    <Page title="Env">
      <Text testTag="window">{env.windowClass}</Text>
    </Page>
  )
}

defineApp({ pages: { root: Root } })
