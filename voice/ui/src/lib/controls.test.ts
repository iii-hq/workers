import { Children, isValidElement, type ReactNode, type ReactElement } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Host, SessionTurnSummaryProps } from '@iii-dev/console-ui'
import { DictationController } from './dictation'
import { startCapture, type CaptureHandle } from './capture'
import { MicButton, useMicPointer } from './mic'
import { createVoiceTurnSummary } from '../turn-summary'
import { speak } from './client'
import { subscribeAutoReplies } from './voice-chat'

// Small deterministic hook host: real control code and controller, no DOM or
// new browser-test dependencies. Browser capture/transport remain simulated.
const hooks = vi.hoisted(() => ({ cursor: 0, values: [] as any[], effects: [] as Array<() => void>, cleanups: [] as Array<() => void> }))
const playback = vi.hoisted(() => ({ state: { phase: 'idle' } as any, play: vi.fn(), enqueue: vi.fn(), stop: vi.fn() }))
vi.mock('react', async (original) => {
  const react = await original<typeof import('react')>()
  return { ...react,
    useState(initial: any) {
      const i = hooks.cursor++
      if (!(i in hooks.values)) hooks.values[i] = typeof initial === 'function' ? initial() : initial
      return [hooks.values[i], (value: any) => { hooks.values[i] = typeof value === 'function' ? value(hooks.values[i]) : value }]
    },
    useRef(initial: any) {
      const i = hooks.cursor++
      return hooks.values[i] ?? (hooks.values[i] = { current: initial })
    },
    useCallback(fn: any) { return fn },
    useSyncExternalStore(_subscribe: any, snapshot: any) { return snapshot() },
    useEffect(effect: () => any, deps: unknown[]) {
      const i = hooks.cursor++
      const previous = hooks.values[i]
      if (!previous || deps.some((value, j) => !Object.is(value, previous[j]))) {
        hooks.values[i] = deps
        hooks.effects.push(() => { hooks.cleanups[i]?.(); hooks.cleanups[i] = effect() })
      }
    },
  }
})
vi.mock('@iii-dev/console-ui', () => ({ IconButton: 'button' }))
vi.mock('./capture', () => ({ startCapture: vi.fn() }))
vi.mock('./client', () => ({
  speak: vi.fn(),
  dictationStart: (iii: any, req: any) => iii.trigger('voice::dictation::start', req),
  dictationStop: (iii: any, req: any) => iii.trigger('voice::dictation::stop', req),
}))
vi.mock('./playback', () => ({ useBrowserPlayback: () => playback }))
vi.mock('./voice-chat', () => ({
  fetchSpokenReply: vi.fn(async () => ({ id: 'reply', text: 'Hello.' })),
  selectedChatText: vi.fn(() => ''), subscribeAutoReplies: vi.fn(() => () => {}),
}))
function render<T>(fn: () => T): T {
  hooks.cursor = 0
  const output = fn()
  for (const effect of hooks.effects.splice(0)) effect()
  return output
}
function all(node: ReactNode): ReturnType<typeof Children.toArray> {
  return Children.toArray(node).flatMap((child) => isValidElement<{ children?: ReactNode }>(child)
    ? [child, ...all(child.props.children)] : [child])
}
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((yes) => { resolve = yes })
  return { promise, resolve }
}
beforeEach(() => {
  vi.clearAllMocks()
  hooks.cursor = 0; hooks.values = []; hooks.effects = []; hooks.cleanups = []
  vi.stubGlobal('window', globalThis)
  vi.stubGlobal('document', { addEventListener: vi.fn(), removeEventListener: vi.fn() })
  playback.state = { phase: 'idle' }
  playback.play.mockImplementation(async (synthesize) => {
    try { await synthesize(); playback.state = { phase: 'speaking' } }
    catch (error) { playback.state = { phase: 'error', message: (error as Error).message } }
  })
})
afterEach(() => {
  for (const cleanup of hooks.cleanups) cleanup?.()
  vi.useRealTimers(); vi.unstubAllGlobals()
})

describe('microphone button gestures with unresolved permission', () => {
  it.each(['click', 'hold-release', 'hold-cancel'])('%s stops without waiting for permission', async (gesture) => {
    vi.useFakeTimers()
    const mic = deferred<CaptureHandle>()
    const requested = deferred<void>()
    vi.mocked(startCapture).mockImplementationOnce(() => { requested.resolve(); return mic.promise })
    const remoteStop = vi.fn(async () => ({ text: '' }))
    const iii = { on: () => () => {}, browserId: 'browser', trigger: (id: string) => id.endsWith('::start')
      ? Promise.resolve({ session_id: 'session' }) : remoteStop() }
    const controller = new DictationController({ iii } as unknown as Host)
    const finish = vi.fn()
    const control = () => MicButton({ pointer: useMicPointer(controller, finish), className: 'mic' })
    const initial = render(control)
    initial.props.onPointerDown()
    if (gesture === 'click') initial.props.onPointerUp()
    else await vi.advanceTimersByTimeAsync(400)
    await requested.promise
    const opening = render(control)
    expect(opening.props['aria-pressed']).toBe(true)
    if (gesture === 'hold-cancel') opening.props.onPointerCancel()
    else {
      if (gesture === 'click') opening.props.onPointerDown()
      opening.props.onPointerUp()
    }
    await vi.advanceTimersByTimeAsync(0)
    expect(remoteStop).toHaveBeenCalledOnce()
    expect(controller.getState().status).toBe('idle')
    expect(render(control).props['aria-pressed']).toBe(false)
    const close = vi.fn(async () => {})
    mic.resolve({ stop: close })
    await vi.advanceTimersByTimeAsync(0)
    expect(close).toHaveBeenCalledOnce()
  })
})

describe('chat read-aloud controls use current availability on every attempt', () => {
  it('queues prepared streaming prose without replaying the final reply', async () => {
    const trigger = vi.fn(async () => ({ text: 'Olá.', max_chunk_chars: 600 }))
    const registration = createVoiceTurnSummary({ iii: { trigger } } as unknown as Host)
    const component = registration.render as (props: SessionTurnSummaryProps) => ReactNode
    const view = () => render(() => component({ sessionId: 'chat', isStreaming: true } as SessionTurnSummaryProps))
    const toggle = all(view()).find((node) => isValidElement<any>(node) && (node.props as { 'aria-pressed'?: boolean })['aria-pressed'] === false) as ReactElement<any>
    toggle.props.onClick(); view()
    const options = vi.mocked(subscribeAutoReplies).mock.calls.at(-1)![2]
    await options.streaming!.prepare('**Olá.**', false)
    expect(trigger).toHaveBeenCalledWith('voice::speech::prepare', { text: '**Olá.**', complete: false })
    options.streaming!.onChunk('Olá.')
    expect(playback.enqueue).toHaveBeenCalledOnce()
    await playback.enqueue.mock.calls[0][0]()
    expect(speak).toHaveBeenCalledWith({ trigger }, { text: 'Olá.', text_format: 'plain' })
    options.onReply({ id: 'reply', text: '**Olá.**' })
    expect(playback.play).not.toHaveBeenCalled()
    options.onStarted()
    expect(playback.stop).toHaveBeenCalled()
  })
  it('can retry after installing a voice without remounting the chat', async () => {
    const registration = createVoiceTurnSummary({ iii: {} } as Host)
    const component = registration.render as (props: SessionTurnSummaryProps) => ReactNode
    const view = () => render(() => component({ sessionId: 'chat', isStreaming: false } as SessionTurnSummaryProps))
    view(); await Promise.resolve(); await Promise.resolve()
    type ButtonProps = { 'aria-label'?: string; disabled?: boolean; onClick: () => void }
    const button = (): ReactElement<ButtonProps> => {
      const found = all(view()).find((node) => isValidElement<ButtonProps>(node)
        && (node.props as ButtonProps)['aria-label'] === 'Read aloud')
      if (!isValidElement(found)) throw new Error('missing read aloud control')
      return found as ReactElement<ButtonProps>
    }
    vi.mocked(speak).mockRejectedValueOnce(new Error('Piper voice is missing; download it in Models'))
    expect(button().props.disabled).toBe(false)
    button().props.onClick()
    await Promise.resolve(); await Promise.resolve()
    expect(playback.state.phase).toBe('error')
    expect(button().props.disabled).toBe(false)
    // A successful download/configuration fix changes the next server result,
    // without a stale doctor snapshot permanently disabling the same component.
    vi.mocked(speak).mockResolvedValueOnce({ backend: 'piper', speech_id: 'ok', played: false, audio_base64: 'wav', mime: 'audio/wav' })
    button().props.onClick()
    await Promise.resolve(); await Promise.resolve()
    expect(playback.state.phase).toBe('speaking')
    expect(speak).toHaveBeenCalledTimes(2)
    expect(speak).toHaveBeenLastCalledWith({}, { text: 'Hello.', text_format: 'markdown' })
  })
})
