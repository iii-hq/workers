/**
 * Chromium missing on the engine's host (design §8.4): the root checks
 * `chromium::status` once, and without one shows the install, its progress
 * (bound to `chromium-install-progress` only while installing and visible)
 * and its failure with the worker's hint.
 */

import {
  Button,
  Column,
  createEffect,
  createSignal,
  EmptyState,
  iii,
  onCleanup,
  onMount,
  Progress,
  Show,
  StatusPanel,
  Text,
} from '@iii-dev/native-ui'
import { browser, type ChromiumStatus, type InstallProgress, listen, TRIGGER } from '../api'
import { megabytes, readable } from '../format'

/** `unknown` until the status answers; the tab list shows meanwhile (the usual case is a Chromium). */
export type ChromiumState = 'unknown' | 'ok' | 'missing' | 'installing' | 'failed'

export interface Chromium {
  state: () => ChromiumState
  progress: () => InstallProgress | null
  status: () => ChromiumStatus | null
  install: () => void
}

const RUNNING = new Set(['resolving', 'downloading', 'extracting', 'verifying'])

/** The Chromium check of the root page; `onReady` runs when an install finishes. */
export function useChromium(onReady: () => void): Chromium {
  const [state, setState] = createSignal<ChromiumState>('unknown')
  const [progress, setProgress] = createSignal<InstallProgress | null>(null)
  const [status, setStatus] = createSignal<ChromiumStatus | null>(null)
  /** Install pressed: the call goes out once the progress binding is ready. */
  let wanted = false

  const apply = (s: ChromiumStatus | null) => {
    if (!s) return
    setStatus(s)
    if (s.found || (s.engine && s.engine !== 'chromium')) return finish(false)
    const job = s.job
    if (job && RUNNING.has(job.phase)) {
      setProgress(job)
      setState('installing')
    } else if (job?.phase === 'failed' && state() !== 'missing') {
      setProgress(job)
      setState('failed')
    } else if (state() !== 'installing') setState('missing')
  }

  const finish = (installed: boolean) => {
    setState('ok')
    setProgress(null)
    if (installed) onReady()
  }

  const fail = (p: InstallProgress) => {
    setProgress(p)
    setState('failed')
  }

  const onProgress = (p: InstallProgress) => {
    if (state() !== 'installing') return
    if (p.phase === 'done') finish(true)
    else if (p.phase === 'failed') fail(p)
    else setProgress(p)
  }

  onMount(() => {
    // A worker without the function (an older one) is treated as having a Chromium.
    browser.chromiumStatus().then(apply, () => {
      if (state() === 'unknown') setState('ok')
    })
  })

  createEffect(() => {
    if (state() !== 'installing' || !iii.env.visible) return
    const off = listen<InstallProgress>(TRIGGER.installProgress, {}, onProgress)
    let alive = true
    onCleanup(() => {
      alive = false
      off()
    })
    void off.ready.then(async () => {
      if (!alive) return
      if (!wanted) {
        // Visible again mid-install: catch up on what the events said meanwhile.
        browser.chromiumStatus().then(
          (s) => {
            if (state() === 'installing') apply(s)
          },
          () => {},
        )
        return
      }
      wanted = false
      try {
        const result = await browser.chromiumInstall()
        if (result?.status === 'already-installed') finish(true)
      } catch (err) {
        fail({ job_id: '', phase: 'failed', bytes_done: 0, error: readable(err) })
      }
    })
  })

  return {
    state,
    progress,
    status,
    install: () => {
      wanted = true
      setProgress({ job_id: '', phase: 'resolving', bytes_done: 0 })
      setState('installing')
    },
  }
}

/** What an install step says, and how far it is (null = indeterminate). */
export function describe(p: InstallProgress): { text: string; value: number | null } {
  const version = p.version ? ` ${p.version}` : ''
  const fraction = p.bytes_total ? Math.min(1, p.bytes_done / p.bytes_total) : null
  switch (p.phase) {
    case 'downloading':
      return {
        text: `Downloading Chrome for Testing${version} · ${megabytes(p.bytes_done, p.bytes_total)}`,
        value: fraction,
      }
    case 'extracting':
      return { text: `Unpacking · ${megabytes(p.bytes_done, p.bytes_total)}`, value: fraction }
    case 'verifying':
      return { text: 'Checking that it starts…', value: null }
    default:
      return { text: 'Finding the latest Chrome for Testing…', value: null }
  }
}

/** The root's body while Chromium is missing, installing or failed. */
export function ChromiumSetup(props: { chromium: Chromium }) {
  const c = props.chromium
  const online = () => iii.env.online
  const support = () => c.status()?.install
  const failure = () => {
    const p = c.progress()
    return [p?.error, p?.hint].filter(Boolean).join('\n\n') || 'The install stopped.'
  }
  return (
    <Column testTag="chromium" weight={1} gap={16} padding={[24, 16]}>
      <Show when={c.state() === 'installing'}>
        <Column testTag="chromium-installing" gap={12}>
          <Text role="heading">Installing Chromium</Text>
          <Progress testTag="chromium-progress" value={describe(c.progress() ?? emptyJob).value} tone="accent" />
          <Text testTag="chromium-phase" role="small" tone="muted">
            {describe(c.progress() ?? emptyJob).text}
          </Text>
        </Column>
      </Show>
      <Show when={c.state() === 'failed'}>
        <StatusPanel
          testTag="chromium-failed"
          variant="alert"
          headline="Chromium didn't install"
          detail={failure()}
          action={
            <Button testTag="chromium-retry" small enabled={online()} onPress={c.install}>
              Retry
            </Button>
          }
        />
      </Show>
      <Show when={c.state() === 'missing'}>
        <EmptyState
          testTag="chromium-missing"
          icon="globe"
          title="Chromium isn't installed on the engine"
          description={
            support()?.supported === false
              ? `The browser worker needs Chrome for Testing on the engine's host. ${support()?.reason ?? ''}`.trim()
              : "The browser worker needs Chrome for Testing on the engine's host."
          }
          actions={
            <Show when={support()?.supported !== false}>
              <Button testTag="chromium-install" kind="primary" icon="download" enabled={online()} onPress={c.install}>
                Install Chromium
              </Button>
            </Show>
          }
        />
      </Show>
    </Column>
  )
}

const emptyJob: InstallProgress = { job_id: '', phase: 'resolving', bytes_done: 0 }
