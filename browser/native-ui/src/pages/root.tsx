/**
 * The root page (design §8.1-§8.4): "Browser" over a ListDetail of the tab
 * list and the open tab. The host arranges it (a detail card on compact,
 * columns on wide); the script only says which tab is open.
 */

import {
  Badge,
  Button,
  createEffect,
  createMemo,
  createSignal,
  EmptyState,
  IconButton,
  iii,
  List,
  ListDetail,
  ListRow,
  type NodeRef,
  on,
  onCleanup,
  Page,
  Row,
  Show,
  Spinner,
  StatusPanel,
  Text,
} from '@iii-dev/native-ui'
import { LIFECYCLE, listen, type Navigated, type SessionStopped, type SessionUpdated } from '../api'
import { readable } from '../format'
import {
  closeTab,
  lifecycle,
  listError,
  loaded,
  newTab,
  refresh,
  refreshSoon,
  rowsOf,
  select,
  selected,
  sessionOf,
  sessions,
  starting,
  type TabRow,
  tabs,
} from '../state'
import { ChromiumSetup, useChromium } from './chromium'
import { TabPage } from './tab'

export function RootPage() {
  const chromium = useChromium(() => void refresh())
  const ready = createMemo(() => {
    const s = chromium.state()
    return s === 'ok' || s === 'unknown'
  })
  const online = () => iii.env.online
  let plus: NodeRef | undefined

  // The rows follow the tabs and the selection.
  createEffect(() => tabs.set(rowsOf(sessions(), selected())))

  // Live only while some page of the app is on screen: the four lifecycle
  // triggers, unfiltered, then one read; visible again binds and reads again.
  createEffect(() => {
    if (!iii.env.visible || !ready()) return
    const offs = LIFECYCLE.map((type) =>
      listen<SessionStopped | SessionUpdated | Navigated>(type, {}, (p) => lifecycle(type, p)),
    )
    let bound = true
    onCleanup(() => {
      bound = false
      for (const off of offs) off()
    })
    void Promise.all(offs.map((off) => off.ready)).then(() => {
      if (bound) void refresh()
    })
  })

  // Back online: what the bus said meanwhile is lost, read again.
  createEffect(
    on(
      () => iii.env.online,
      (now, before) => {
        if (now && before === false && iii.env.visible) refreshSoon(0)
      },
    ),
  )

  const newTabMenu = async () => {
    const choice = await iii.ui.actionMenu(plus ?? null, {
      actions: [
        { label: 'New tab', icon: 'plus' },
        { label: 'New incognito tab', icon: 'eye-off' },
      ],
    })
    if (choice >= 0) await newTab(choice === 1)
  }

  return (
    <Page
      testTag="browser"
      title="Browser"
      actions={
        <Show when={ready()}>
          <IconButton
            testTag="new-tab"
            ref={(node) => {
              plus = node
            }}
            icon="plus"
            label="New tab"
            enabled={online() && !starting()}
            onPress={() => void newTab()}
            onLongPress={() => void newTabMenu()}
          />
          <IconButton
            testTag="browser-menu"
            icon="ellipsis-vertical"
            label="More"
            enabled={online()}
            onPress={() => void newTabMenu()}
          />
        </Show>
      }
      top={
        <Show when={ready() && listError()}>
          <StatusPanel
            testTag="list-error"
            variant="alert"
            headline="Couldn't read the tabs"
            detail={listError()}
            action={
              <Button testTag="list-retry" small enabled={online()} onPress={() => void refresh()}>
                Retry
              </Button>
            }
          />
        </Show>
      }
    >
      <Show when={ready()} fallback={<ChromiumSetup chromium={chromium} />}>
        <ListDetail
          testTag="tabs-split"
          listTitle="Tabs"
          detailFullHeight
          showDetail={selected() !== null}
          onDismiss={() => select(null)}
          list={<TabList />}
          detail={
            <Show when={selected()} keyed>
              {(id: string) => <TabPage id={id} />}
            </Show>
          }
          emptyDetail={
            <EmptyState
              testTag="no-tab-open"
              icon="globe"
              title="No tab open"
              description="Pick a tab from the list, or open a new one."
            />
          }
        />
      </Show>
    </Page>
  )
}

function TabList() {
  const online = () => iii.env.online
  const [pulling, setPulling] = createSignal(false)
  const pull = async () => {
    setPulling(true)
    await refresh()
    setPulling(false)
  }
  const close = (item: TabRow) => void closeTab(item.key)
  const menu = (item: TabRow) => void tabMenu(item)
  return (
    <List
      testTag="tabs"
      weight={1}
      source={tabs}
      refreshing={pulling()}
      onRefresh={() => void pull()}
      templates={[
        (row) => (
          <ListRow
            title={row.title}
            subtitle={row.subtitle}
            subtitleMono
            icon={row.icon}
            selected={row.selected}
            onPress={(item: TabRow) => select(item.key)}
            onLongPress={menu}
            trailing={<IconButton testTag="tab-close" icon="x" label="Close tab" enabled={online()} onPress={close} />}
          />
        ),
        (row) => <Text role="micro" tone="muted" padding={[16, 16, 4, 16]} text={row.title} />,
        (row) => (
          <ListRow
            title={row.title}
            subtitle={row.subtitle}
            subtitleMono
            icon={row.icon}
            tone="faint"
            selected={row.selected}
            onPress={(item: TabRow) => select(item.key)}
            onLongPress={menu}
            trailing={
              <Row gap={4}>
                <Badge text="Asleep" />
                <IconButton testTag="tab-close" icon="x" label="Close tab" enabled={online()} onPress={close} />
              </Row>
            }
          />
        ),
      ]}
      empty={
        <Show when={loaded()} fallback={<Spinner testTag="tabs-loading" padding={24} />}>
          <EmptyState
            testTag="no-tabs"
            icon="globe"
            title="No tabs"
            description="Open a page in the engine's browser."
            actions={[
              <Button
                testTag="empty-new-tab"
                kind="primary"
                icon="plus"
                enabled={online()}
                onPress={() => void newTab()}
              >
                New tab
              </Button>,
              <Button testTag="empty-new-incognito" icon="eye-off" enabled={online()} onPress={() => void newTab(true)}>
                New incognito tab
              </Button>,
            ]}
          />
        </Show>
      }
    />
  )
}

/** The long press on a tab row. */
async function tabMenu(item: TabRow): Promise<void> {
  const url = sessionOf(item.key)?.url ?? ''
  const choice = await iii.ui.actionMenu(null, {
    title: item.title,
    subtitle: item.subtitle,
    actions: [
      { label: 'Open', icon: 'globe' },
      { label: 'Copy URL', icon: 'copy' },
      { label: 'Share URL', icon: 'share-2' },
      { label: 'Close tab', icon: 'x' },
    ],
  })
  // The tab may have stopped while the menu was open: nothing goes to a dead session.
  if (choice < 0 || !sessionOf(item.key)) return
  try {
    if (choice === 0) select(item.key)
    else if (choice === 1) {
      await iii.clipboard.writeText(url)
      iii.ui.toast('Copied the address')
    } else if (choice === 2) await iii.share({ text: url, title: item.title })
    else if (choice === 3) await closeTab(item.key)
  } catch (err) {
    iii.ui.toast(readable(err), 'error')
  }
}
