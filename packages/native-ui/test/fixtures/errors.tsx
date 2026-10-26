/** Errors: a throwing handler and trigger are reported, not fatal; a page that cannot render is fatal. */
import { Button, Column, createSignal, defineApp, Page, Text, useTrigger } from '@iii-dev/native-ui'

function Root() {
  const [n, setN] = createSignal(0)
  useTrigger(
    'demo::boom',
    () => ({}),
    () => {
      throw new Error('trigger exploded')
    },
  )
  return (
    <Page title="Errors">
      <Column>
        <Text testTag="n">{String(n())}</Text>
        <Button
          testTag="throw"
          text="Throw"
          onPress={() => {
            setN(n() + 1)
            throw new TypeError('handler exploded')
          }}
        />
        <Button testTag="ok" text="OK" onPress={() => setN(n() + 1)} />
      </Column>
    </Page>
  )
}

function Broken(): never {
  throw new Error('cannot render')
}

defineApp({ pages: { root: Root, broken: Broken } })
