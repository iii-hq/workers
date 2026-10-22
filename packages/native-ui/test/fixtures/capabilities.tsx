/** Each button exercises one capability; the outcome goes to the "out" Text. */
import { Button, Column, createSignal, defineApp, IiiError, iii, Page, Text } from '@iii-dev/native-ui'

function Root() {
  const [out, setOut] = createSignal('')
  const show = (p: Promise<unknown>) =>
    p.then(
      (v) => setOut(`ok ${JSON.stringify(v ?? null)}`),
      (e: IiiError) => setOut(`error ${e instanceof IiiError ? e.code : 'other'}: ${e.message}`),
    )
  const actions: Record<string, () => void> = {
    'call-own': () => show(iii.call('demo::echo', { a: 1 }, { timeoutMs: 999_999 })),
    'call-missing': () => show(iii.call('demo::missing')),
    'call-other': () => show(iii.call('state::get', {})),
    'call-engine': () => show(iii.call('engine::functions::list', {})),
    'bind-other': () => {
      try {
        iii.on('state::changed', {}, () => {})
        setOut('bound')
      } catch (e) {
        setOut(`threw ${(e as IiiError).code}: ${(e as Error).message}`)
      }
    },
    toast: () => iii.ui.toast('Saved', 'success'),
    confirm: () => show(iii.ui.confirm({ title: 'Clear?', confirm: 'Clear', destructive: true })),
    prompt: () => show(iii.ui.prompt({ title: 'Name', confirm: 'OK', keyboard: 'url', mono: true })),
    clipboard: () => show(iii.clipboard.writeText('copied')),
    'open-http': () => show(iii.openUrl('https://iii.dev')),
    'open-file': () => show(iii.openUrl('file:///etc/passwd')),
    share: () => show(iii.share({ text: 'hello', title: 'Hi' })),
    save: () => show(iii.saveFile({ data: 'aGk=', mime: 'text/plain', filename: 'hi.txt' })),
    storage: () =>
      show(
        iii.storage
          .set('k', { v: 1 })
          .then(() => iii.storage.get('k'))
          .then((v) => iii.storage.keys().then((keys) => ({ v, keys }))),
      ),
    'storage-long-key': () => show(iii.storage.get('k'.repeat(200))),
    'push-unknown': () => show(iii.nav.push('nowhere')),
    app: () => setOut(`${iii.app.key} ${iii.app.ns}`),
    report: () => iii.reportError(new Error('soft failure')),
  }
  return (
    <Page title="Caps">
      <Column>
        <Text testTag="out">{out()}</Text>
        {Object.keys(actions).map((name) => (
          <Button testTag={name} text={name} onPress={actions[name]} />
        ))}
      </Column>
    </Page>
  )
}

defineApp({ pages: { root: Root } })
