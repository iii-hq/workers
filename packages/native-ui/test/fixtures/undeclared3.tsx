/** Catalogue 3 pieces in an app that declares 2. */
import { Button, Card, Chart, Column, defineApp, iii, Text } from '@iii-dev/native-ui'

function Root() {
  return (
    <Column>
      <Card tone="attention">
        <Text>Old app</Text>
      </Card>
      <Chart series={[1, 2]} />
      <Button testTag="toast" onPress={() => iii.ui.toast('Saved', { action: 'View' })}>
        Save
      </Button>
    </Column>
  )
}

defineApp({ catalog: 2, pages: { root: Root } })
