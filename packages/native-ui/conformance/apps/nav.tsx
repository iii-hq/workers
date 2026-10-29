/**
 * §5.7 "Navegação": the root pushes "console" with {session: "s1"}; the card
 * leaves by Back (result null) or by its own Done (a result for the root).
 */
import { Button, Column, createSignal, defineApp, iii, Page, Text, usePage } from '@iii-dev/native-ui'

function Root() {
  const [last, setLast] = createSignal('No console yet')
  const open = () => {
    iii.nav.push('console', { session: 's1' }).then((result) => setLast(`Console closed: ${JSON.stringify(result)}`))
  }
  return (
    <Page title="Browser">
      <Column>
        <Text>{last()}</Text>
        <Button text="Console" onPress={open} />
      </Column>
    </Page>
  )
}

function Console(props: { params: { session?: unknown } }) {
  const page = usePage()
  return (
    <Page title="Console" subtitle={String(props.params.session)} subtitleMono>
      <Button text="Done" onPress={() => page.pop({ picked: 'error' })} />
    </Page>
  )
}

defineApp({ pages: { root: Root, console: { component: Console, fullScreen: true } } })
