/** §5.7 "Um contador": Page "Demo" > Column gap 12, padding 16 > Text "Count 0" (title), Button "Add" (primary, press). */
import { Button, Column, createSignal, defineApp, Page, Text } from '@iii-dev/native-ui'

function Counter() {
  const [count, setCount] = createSignal(0)
  return (
    <Page title="Demo">
      <Column gap={12} padding={16}>
        <Text role="title">Count {count()}</Text>
        <Button kind="primary" onPress={() => setCount(count() + 1)}>
          Add
        </Button>
      </Column>
    </Page>
  )
}

defineApp({ pages: { root: Counter } })
