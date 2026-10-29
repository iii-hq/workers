/** Catalogue 2 pieces in an app that declares no catalogue (so 1). */
import { Chip, Column, defineApp, Markdown, Text } from '@iii-dev/native-ui'

function Root() {
  return (
    <Column>
      <Text>Old app</Text>
      <Markdown text="**new**" />
      <Chip hue="rose">Rose</Chip>
    </Column>
  )
}

defineApp({ pages: { root: Root } })
