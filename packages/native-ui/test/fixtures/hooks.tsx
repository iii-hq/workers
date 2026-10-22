/** useCall, useTrigger with `enabled`, onEnv, usePage().replace and the other pages. */
import {
  Button,
  Column,
  createSignal,
  defineApp,
  iii,
  Page,
  Show,
  Switch,
  Text,
  useCall,
  useEnv,
  usePage,
  useTrigger,
} from '@iii-dev/native-ui'

const [envLog, setEnvLog] = createSignal<string[]>([])
iii.onEnv((_env, changed) => setEnvLog([...envLog(), changed.join(',')]))

function Root() {
  const [query, setQuery] = createSignal<string | undefined>('one')
  const [live, setLive] = createSignal(true)
  const [events, setEvents] = createSignal(0)
  const [result] = useCall<{ echo: string }>('demo::echo', () => {
    const q = query()
    return q === undefined ? undefined : { q }
  })
  useTrigger(
    'demo::tick',
    () => ({ every: 1 }),
    () => setEvents(events() + 1),
    { enabled: live },
  )
  const env = useEnv()
  return (
    <Page title="Hooks">
      <Column>
        <Text testTag="result">{result()?.echo ?? 'loading'}</Text>
        <Show when={result()}>{(r) => <Text testTag="echo">{`echo ${r().echo}`}</Text>}</Show>
        <Text testTag="events">{String(events())}</Text>
        <Text testTag="visible">{env.visible ? 'visible' : 'hidden'}</Text>
        <Text testTag="env-log">{envLog().join(' | ')}</Text>
        <Button testTag="two" text="Two" onPress={() => setQuery('two')} />
        <Button testTag="none" text="None" onPress={() => setQuery(undefined)} />
        <Switch testTag="live" checked={live()} onChange={setLive} />
        <Button testTag="detail" text="Detail" onPress={() => iii.nav.push('detail', { n: 1 })} />
      </Column>
    </Page>
  )
}

function Detail() {
  const page = usePage()
  return (
    <Page title={`Detail ${JSON.stringify(page.params)}`}>
      <Button testTag="swap" text="Swap" onPress={() => page.replace('other', { from: page.inst })} />
    </Page>
  )
}

function Other() {
  const page = usePage()
  return (
    <Page title="Other">
      <Text testTag="from">{String(page.params.from)}</Text>
      <Button testTag="home" text="Home" onPress={() => iii.nav.popToRoot()} />
    </Page>
  )
}

defineApp({ pages: { root: Root, detail: Detail, other: { component: Other, modal: true } } })
