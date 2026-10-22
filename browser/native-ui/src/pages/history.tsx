/**
 * The history of a tab (design §8.4), its own card: a search, the visits
 * newest first (re-read on each navigation of the tab), and a tap that
 * takes the tab there: the card pops with the URL and the tab page, which
 * pushed it, navigates (so its loading bar shows).
 */

import {
  Button,
  Column,
  createEffect,
  createList,
  createSignal,
  EmptyState,
  iii,
  type JsonObject,
  List,
  ListRow,
  onCleanup,
  Page,
  Show,
  Spinner,
  StatusPanel,
  Text,
  TextField,
  untrack,
  usePage,
} from '@iii-dev/native-ui'
import { displayUrl, tabTitle } from '../address'
import { browser, listen, type Navigated, TRIGGER, type Visit } from '../api'
import { clock, readable } from '../format'
import { sessionOf } from '../state'
import { copyMenu } from './console'

export interface VisitRow {
  key: string
  title: string
  subtitle: string
  time: string
  url: string
}

export function visitRows(visits: readonly Visit[]): VisitRow[] {
  return visits.map((v) => ({
    key: `${v.timestamp}|${v.url}`,
    title: tabTitle(v.title, v.url),
    subtitle: displayUrl(v.url) || v.url,
    time: clock(v.timestamp),
    url: v.url,
  }))
}

export function HistoryPage(props: { params: JsonObject }) {
  const session = String(props.params.session ?? '')
  const page = usePage()
  const rows = createList<VisitRow>({ key: 'key' })
  const [text, setText] = createSignal('')
  const [query, setQuery] = createSignal('')
  const [error, setError] = createSignal<string | null>(null)
  const [loaded, setLoaded] = createSignal(false)
  let timer: number | undefined
  let asked = 0
  onCleanup(() => clearTimeout(timer))

  const load = async (q: string) => {
    const ask = ++asked
    try {
      const visits = await browser.visits(session, q)
      if (ask !== asked) return
      rows.set(visitRows(visits))
      setError(null)
    } catch (err) {
      if (ask === asked) setError(readable(err))
    } finally {
      if (ask === asked) setLoaded(true)
    }
  }

  const typed = (value: string) => {
    setText(value)
    clearTimeout(timer)
    timer = setTimeout(() => setQuery(value.trim()), 300)
  }

  // Read on open, on each search, and when visible again.
  createEffect(() => {
    if (!iii.env.visible) return
    const q = query()
    untrack(() => void load(q))
  })

  // A navigation of the tab adds a visit.
  createEffect(() => {
    if (!iii.env.visible) return
    const off = listen<Navigated>(TRIGGER.navigated, { session_id: session }, (p) => {
      if (p.session_id === session) void load(untrack(query))
    })
    onCleanup(off)
  })

  const name = () => {
    const info = sessionOf(session)
    return info ? tabTitle(info.title, info.url) : null
  }

  return (
    <Page
      testTag="history"
      title="History"
      subtitle={name()}
      top={
        <Column gap={8} padding={[8, 12]}>
          <TextField
            testTag="history-search"
            value={text()}
            onChange={typed}
            placeholder="Search history"
            leadingIcon="search"
            keyboard="search"
            imeAction="search"
            clearable
          />
          <Show when={error()}>
            {(message: () => string) => (
              <StatusPanel
                testTag="history-error"
                variant="alert"
                headline="Couldn't read the history"
                detail={message()}
                action={
                  <Button testTag="history-retry" small onPress={() => void load(query())}>
                    Retry
                  </Button>
                }
              />
            )}
          </Show>
        </Column>
      }
    >
      <List
        testTag="history-list"
        weight={1}
        source={rows}
        templates={[
          (row) => (
            <ListRow
              title={row.title}
              subtitle={row.subtitle}
              subtitleMono
              icon="history"
              onPress={(item: VisitRow) => page.pop({ url: item.url })}
              onLongPress={(item: VisitRow) => void copyMenu(item.title, 'Copy URL', item.url)}
              trailing={<Text role="codeSmall" tone="faint" text={row.time} />}
            />
          ),
        ]}
        empty={
          <Show when={loaded()} fallback={<Spinner padding={24} />}>
            <EmptyState
              testTag="history-empty"
              icon="history"
              title={query() ? 'Nothing matches' : 'No history yet'}
              description={query() ? undefined : 'The pages this tab visits show up here.'}
            />
          </Show>
        }
      />
    </Page>
  )
}
