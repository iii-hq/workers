/** What the VM offers: the globals, no eval, the fake clock's timers. */
import { Button, Column, createSignal, defineApp, Page, Text } from '@iii-dev/native-ui'

const g = globalThis as Record<string, unknown>

function attempt(fn: () => unknown): string {
  try {
    fn()
    return 'ran'
  } catch (e) {
    return (e as Error).name
  }
}

const report = {
  hostSend: typeof g.__host_send,
  rawRequest: typeof g.__iiiui_request,
  vmReceive: typeof g.__vm_receive,
  // biome-ignore lint/complexity/noCommaOperator: an indirect eval, on purpose
  // biome-ignore lint/security/noGlobalEval: the test checks that eval throws
  eval: attempt(() => (0, eval)('1')),
  newFunction: attempt(() => new Function('return 1')),
  ctor: attempt(() => (() => {}).constructor('return 1')),
  asyncCtor: attempt(() => Object.getPrototypeOf(async () => {}).constructor('return 1')),
  missing: ['URL', 'TextEncoder', 'atob', 'fetch', 'document', 'Intl', 'WebSocket'].filter((k) => k in g),
  present: [
    'console',
    'setTimeout',
    'clearTimeout',
    'setInterval',
    'clearInterval',
    'queueMicrotask',
    'performance',
  ].filter((k) => typeof g[k] === 'object' || typeof g[k] === 'function'),
  regexp: /a+b/.test('aab'),
}

function Root() {
  const [ticks, setTicks] = createSignal<string[]>([])
  const note = (s: string) => setTicks([...ticks(), s])
  let interval: number | undefined
  return (
    <Page title="Sandbox">
      <Column>
        <Text testTag="report">{JSON.stringify(report)}</Text>
        <Text testTag="ticks">{ticks().join(',')}</Text>
        <Button
          testTag="timers"
          text="Timers"
          onPress={() => {
            const t0 = performance.now()
            setTimeout(() => note(`t100@${performance.now() - t0}`), 100)
            setTimeout(() => note(`t50@${performance.now() - t0}`), 50)
            const cancelled = setTimeout(() => note('never'), 10)
            clearTimeout(cancelled)
            interval = setInterval(() => note('i'), 1)
            setTimeout(() => clearInterval(interval), 9)
            queueMicrotask(() => note('micro'))
          }}
        />
        <Button
          testTag="spin"
          text="Spin"
          onPress={() => {
            for (;;) {}
          }}
        />
        <Button
          testTag="hog"
          text="Hog"
          onPress={() => {
            const keep: number[][] = []
            for (;;) keep.push(new Array(100_000).fill(keep.length))
          }}
        />
      </Column>
    </Page>
  )
}

defineApp({ pages: { root: Root } })
