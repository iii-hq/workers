/**
 * §5.7 "Um pedido de chamada, uma assinatura, um erro": a press calls
 * browser::sessions::list and binds browser::session-started; the trigger
 * updates the page; a second press gets the host's forbidden error.
 */
import { Button, Column, createSignal, defineApp, iii, Page, Text } from '@iii-dev/native-ui'

function Sessions() {
  const [status, setStatus] = createSignal('Not loaded')
  const load = () => {
    iii.call<{ sessions: unknown[] }>('browser::sessions::list', {}).then((r) => setStatus(`${r.sessions.length} tabs`))
    iii.on<{ session_id: string }>('browser::session-started', {}, (p) => setStatus(`Started ${p.session_id}`))
  }
  const stop = () => {
    iii.call('browser::sessions::stop', { session_id: 's1' }).catch((err: Error) => setStatus(err.message))
  }
  return (
    <Page title="Browser">
      <Column gap={8}>
        <Text>{status()}</Text>
        <Button text="Load" onPress={load} />
        <Button text="Stop" kind="destructive" onPress={stop} />
      </Column>
    </Page>
  )
}

defineApp({ pages: { root: Sessions } })
