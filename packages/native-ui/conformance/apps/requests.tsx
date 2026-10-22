/**
 * §5.7 "Um pedido de chamada, uma assinatura, um erro": a press calls
 * browser::sessions::list and binds browser::session-started; the trigger
 * updates the page; a second press calls engine::functions::list, outside
 * the namespace, and gets the host's forbidden error. `iii.call` refuses
 * such a call before the wire, so the flow sends it raw (the harness's
 * `rawRequests`, test-only) to show what the host answers.
 */
import { Button, Column, createSignal, defineApp, iii, Page, Text } from '@iii-dev/native-ui'

type RawRequest = (method: number, args: Record<string, unknown>) => Promise<unknown>
const CALL = 1

function Sessions() {
  const [status, setStatus] = createSignal('Not loaded')
  const load = () => {
    iii.call<{ sessions: unknown[] }>('browser::sessions::list', {}).then((r) => setStatus(`${r.sessions.length} tabs`))
    iii.on<{ session_id: string }>('browser::session-started', {}, (p) => setStatus(`Started ${p.session_id}`))
  }
  const outside = () => {
    const raw = (globalThis as { __iiiui_request?: RawRequest }).__iiiui_request
    if (!raw) return setStatus('No raw requests in this VM')
    raw(CALL, { fn: 'engine::functions::list', payload: {} }).catch((err: Error) => setStatus(err.message))
  }
  return (
    <Page title="Browser">
      <Column gap={8}>
        <Text>{status()}</Text>
        <Button text="Load" onPress={load} />
        <Button text="Engine" kind="destructive" onPress={outside} />
      </Column>
    </Page>
  )
}

defineApp({ pages: { root: Sessions } })
