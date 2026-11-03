/** Errors: a throwing handler and observer are reported, not fatal; a page that cannot render is fatal. */
import { Button, Column, createSignal, defineApp, Page, Text, useObserve } from '@iii-dev/native-ui'

function Root() {
  const [n, setN] = createSignal(0)
  useObserve(
    'demo::on-boom',
    () => ({}),
    () => {
      throw new Error('observer exploded')
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
