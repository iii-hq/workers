/**
 * The open tab (design §8.2-§8.4): the detail Page of the root's
 * ListDetail. The live page is a RemoteScreen the host drives on its own
 * (frames, gestures, the viewer lease, the resize); the script gives it the
 * tab and the wire, and owns the address bar, the loading bar, the handoff
 * and error panels and the menus.
 */

import {
  Button,
  Column,
  createEffect,
  createSignal,
  EmptyState,
  IconButton,
  iii,
  type MenuAction,
  type NodeRef,
  onCleanup,
  onMount,
  Page,
  Progress,
  RemoteScreen,
  Show,
  StatusPanel,
} from '@iii-dev/native-ui'
import { hostOf, isWebUrl, tabTitle } from '../address'
import { browser, type HandoffRequested, type HandoffResolved, listen, TRIGGER, WIRE } from '../api'
import { readable, stamp } from '../format'
import { closeTab, framed, type Handoff, history, navigate, type OpenTab, openTab, select, sessionOf } from '../state'
import { Bars } from './bars'

/** Why a tab stopped, in words (own entries only: a reason is the worker's text). */
const CLOSED = new Map<string, string>([
  ['crashed', 'it crashed'],
  ['expired', 'it expired'],
  ['idle', 'it was idle'],
  ['stopped', 'elsewhere'],
])

export function TabPage(props: { id: string }) {
  const tab = openTab(props.id)
  const readOnly = () => sessionOf(tab.id)?.read_only === true
  const wide = () => iii.env.windowClass === 'wide'
  const online = () => iii.env.online
  const [live, setLive] = createSignal(true)
  const [viewError, setViewError] = createSignal<string | null>(null)
  let menuButton: NodeRef | undefined

  // A handoff is for the person looking at the tab: bound while it is on screen.
  createEffect(() => {
    if (!iii.env.visible) return
    const config = { session_id: tab.id }
    const offs = [
      listen<HandoffRequested>(TRIGGER.handoffRequested, config, (p) => {
        if (p.session_id !== tab.id || !p.handoff_id) return
        tab.setHandoffs((queue) =>
          queue.some((h) => h.id === p.handoff_id)
            ? queue
            : [...queue, { id: p.handoff_id, instructions: p.instructions ?? '' }],
        )
      }),
      listen<HandoffResolved>(TRIGGER.handoffResolved, config, (p) => {
        if (p.session_id === tab.id) tab.setHandoffs((queue) => queue.filter((h) => h.id !== p.handoff_id))
      }),
    ]
    onCleanup(() => {
      for (const off of offs) off()
    })
  })

  // The zoom of an awake page (a waking one reloads at 100 %).
  onMount(() => {
    if (!tab.waking) browser.zoom(tab.id, 'read').then(tab.setZoom, () => {})
  })

  const handoff = () => tab.handoffs()[0] ?? null
  const done = (id: string) => {
    tab.setHandoffs((queue) => queue.filter((h) => h.id !== id))
    browser.handoffDone(tab.id, id).catch((err) => iii.ui.toast(readable(err), 'error'))
  }

  // The live view stopped: the person can try again (enabled off, then on).
  const restartView = () => {
    setViewError(null)
    setLive(false)
    setTimeout(() => setLive(true), 100)
  }

  const zoom = async (action: 'in' | 'out' | 'reset') => {
    try {
      tab.setZoom(await browser.zoom(tab.id, action))
    } catch (err) {
      iii.ui.toast(readable(err), 'error')
    }
  }

  const screenshot = async () => {
    const shot = await browser.screenshot(tab.id)
    const image = shot?.content?.find((c) => c.type === 'image')
    const data = image && 'data' in image ? image.data : undefined
    if (!data) throw new Error('No image came back')
    const mime = (image && 'mime' in image && image.mime) || 'image/png'
    const file = {
      data,
      mime,
      filename: `screenshot-${hostOf(tab.url()).replace(/[^a-z0-9.-]+/g, '-') || 'page'}-${stamp(Date.now())}.${mime.includes('jpeg') ? 'jpg' : 'png'}`,
    }
    const choice = await iii.ui.actionMenu(menuButton ?? null, {
      title: 'Screenshot',
      actions: [
        { label: 'Share', icon: 'share-2' },
        { label: 'Save to Downloads', icon: 'download' },
      ],
    })
    if (choice === 0) await iii.share(file)
    else if (choice === 1) {
      const saved = await iii.saveFile(file)
      iii.ui.toast(`Saved to ${saved.path}`, 'success')
    }
  }

  const clearSiteData = async () => {
    const host = hostOf(tab.url()) || 'this site'
    const ok = await iii.ui.confirm({
      title: `Clear data for ${host}?`,
      message: 'Cookies, cache and storage of this site.',
      confirm: 'Clear',
      destructive: true,
    })
    if (!ok) return
    const cleared = await browser.clearData(tab.id)
    iii.ui.toast(cleared.length ? `Cleared ${cleared.join(', ')}` : 'Nothing to clear for this site')
    await history(tab, 'reload')
  }

  const devtools = (page: 'console' | 'network') => iii.nav.push(page, { session: tab.id })

  const visits = async () => {
    const picked = await iii.nav.push<{ url: string }>('history', { session: tab.id })
    if (picked && typeof picked.url === 'string') await navigate(tab, picked.url)
  }

  const copyUrl = async () => {
    await iii.clipboard.writeText(tab.url())
    iii.ui.toast('Copied the address')
  }

  const openOutside = async () => {
    if (!isWebUrl(tab.url())) iii.ui.toast("This page can't open outside the engine's browser")
    else await iii.openUrl(tab.url())
  }

  const menu = async () => {
    const items: [MenuAction, () => unknown][] = [
      [{ label: 'Find in page', icon: 'search' }, () => tab.setFind(tab.find() ?? { query: '', count: 0, index: 0 })],
      [{ label: 'Zoom in', icon: 'zoom-in' }, () => zoom('in')],
      [{ label: 'Zoom out', icon: 'zoom-out' }, () => zoom('out')],
      [{ label: 'Actual size', hint: `${tab.zoom()}%` }, () => zoom('reset')],
      [{ label: 'Screenshot', icon: 'camera' }, screenshot],
      [{ label: 'Console', icon: 'terminal' }, () => devtools('console')],
      [{ label: 'Network', icon: 'network' }, () => devtools('network')],
      [{ label: 'History', icon: 'history' }, visits],
      [{ label: 'Copy URL', icon: 'copy' }, copyUrl],
      [{ label: 'Share URL', icon: 'share-2' }, () => iii.share({ text: tab.url(), title: title() })],
      [{ label: 'Open in browser', icon: 'external-link' }, openOutside],
      [{ label: 'Clear site data…', icon: 'trash', destructive: true }, clearSiteData],
      [{ label: 'Close tab', icon: 'x' }, () => closeTab(tab.id)],
    ]
    const choice = await iii.ui.actionMenu(menuButton ?? null, {
      title: title(),
      subtitle: hostOf(tab.url()) || undefined,
      actions: items.map(([action]) => action),
    })
    await run(tab, items[choice]?.[1])
  }

  // A long press on the page, at (x, y) in page coordinates.
  const pageMenu = async (x: unknown, y: unknown) => {
    // Clicks need a point on the page; without one, only the address is offered.
    const px = typeof x === 'number' && Number.isFinite(x) ? x : null
    const py = typeof y === 'number' && Number.isFinite(y) ? y : null
    const items: [MenuAction, () => unknown][] =
      readOnly() || px === null || py === null
        ? [[{ label: 'Copy page URL', icon: 'copy' }, copyUrl]]
        : [
            [{ label: 'Right-click here', icon: 'mouse-pointer-click' }, () => browser.click(tab.id, px, py, 'right')],
            [
              { label: 'Type here', icon: 'type' },
              async () => {
                await browser.click(tab.id, px, py, 'left')
                tab.setKeyboard(true)
              },
            ],
            [{ label: 'Copy page URL', icon: 'copy' }, copyUrl],
          ]
    const choice = await iii.ui.actionMenu(null, { actions: items.map(([action]) => action) })
    await run(tab, items[choice]?.[1])
  }

  const title = () => tabTitle(tab.title(), tab.url())
  const closedTitle = () => {
    const why = CLOSED.get(tab.closed() ?? '')
    return why ? `This tab was closed (${why})` : 'This tab was closed'
  }

  // Created once: the page moves this node between its top and bottom slots.
  const bars = <Bars tab={tab} />

  return (
    <Page
      testTag="tab"
      title={title()}
      subtitle={hostOf(tab.url()) || null}
      subtitleMono
      actions={
        <IconButton
          testTag="tab-menu"
          ref={(node) => {
            menuButton = node
          }}
          visible={!tab.closed()}
          icon="ellipsis-vertical"
          label="Page menu"
          enabled={online()}
          onPress={() => void menu()}
        />
      }
      top={
        <Column testTag="tab-top">
          <Column testTag="bars-top">{wide() ? bars : null}</Column>
          <Progress testTag="loading" visible={tab.loading() && !tab.closed()} tone="accent" />
          <Show when={!tab.closed() && handoff()}>
            {(h: () => Handoff) => (
              <StatusPanel
                testTag="handoff"
                variant="warn"
                headline="The agent needs you"
                detail={h().instructions || 'Finish the step on the page, then tap I’m done.'}
                padding={[8, 12]}
                action={
                  <Button testTag="handoff-done" small kind="primary" onPress={() => done(h().id)}>
                    I'm done
                  </Button>
                }
              />
            )}
          </Show>
          <Show when={!tab.closed() && tab.error()}>
            {(e: () => { url: string; detail: string }) => (
              <StatusPanel
                testTag="load-error"
                variant="alert"
                headline={`Couldn't load ${hostOf(e().url) || e().url}`}
                detail={e().detail}
                padding={[8, 12]}
                action={[
                  <Button testTag="load-retry" small enabled={online()} onPress={() => void navigate(tab, e().url)}>
                    Retry
                  </Button>,
                  <IconButton testTag="load-dismiss" icon="x" label="Dismiss" onPress={() => tab.setError(null)} />,
                ]}
              />
            )}
          </Show>
          <Show when={!tab.closed() && viewError()}>
            {(message: () => string) => (
              <StatusPanel
                testTag="view-error"
                variant="alert"
                headline="The live view stopped"
                detail={message()}
                padding={[8, 12]}
                action={
                  <Button testTag="view-retry" small enabled={online()} onPress={restartView}>
                    Retry
                  </Button>
                }
              />
            )}
          </Show>
        </Column>
      }
      bottom={wide() ? null : bars}
    >
      <Show
        when={!tab.closed()}
        fallback={
          <EmptyState
            testTag="tab-closed"
            icon="x"
            title={closedTitle()}
            actions={
              <Button testTag="back-to-tabs" onPress={() => select(null)}>
                Back to tabs
              </Button>
            }
          />
        }
      >
        <RemoteScreen
          testTag="screen"
          weight={1}
          session={tab.id}
          wire={WIRE}
          mobile
          mode={readOnly() ? 'view' : 'interact'}
          enabled={live()}
          keyboard={tab.keyboard()}
          placeholder={tab.waking ? 'Waking the tab…' : 'Loading…'}
          onReady={() => {
            tab.waking = false
            setViewError(null)
          }}
          onFrame={(width: number, _height: number, modeChanged?: unknown) => framed(tab, width, modeChanged)}
          onError={(message: string) => setViewError(message || 'The page stopped streaming.')}
          onKeyboard={(open: boolean) => tab.setKeyboard(open === true)}
          onLongPress={(x: number, y: number) => void pageMenu(x, y)}
        />
      </Show>
    </Page>
  )
}

/** Runs a menu choice; a failure is a toast in words. */
async function run(tab: OpenTab, action: (() => unknown) | undefined): Promise<void> {
  // The tab may have stopped while the menu was open: nothing goes to a dead session.
  if (!action || tab.closed()) return
  try {
    await action()
  } catch (err) {
    iii.ui.toast(readable(err), 'error')
  }
}
