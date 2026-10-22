/**
 * The console of a tab (design §8.4), its own card: a level filter, a
 * pattern, and the entries (seed then live, newest 500). A long press
 * copies an entry.
 */

import {
  type Bound,
  Button,
  Column,
  createSignal,
  EmptyState,
  IconButton,
  iii,
  type JsonObject,
  List,
  onCleanup,
  Page,
  Row,
  Segmented,
  Show,
  Spinner,
  StatusPanel,
  Text,
  TextField,
  type Tone,
} from '@iii-dev/native-ui'
import { tabTitle } from '../address'
import { browser, type ConsoleEntry, TRIGGER } from '../api'
import { createFeed, patternTest, SEED_LIMIT } from '../feed'
import { clock, readable } from '../format'
import { sessionOf } from '../state'

/** Segmented options and the level each asks the worker for. */
const LEVELS = [
  ['All', ''],
  ['Errors', 'error'],
  ['Warnings', 'warning'],
  ['Info', 'info'],
  ['Debug', 'debug'],
] as const

export interface ConsoleRow {
  key: string
  /** The template: 0 log/info, 1 warning, 2 error, 3 debug (each with its tone). */
  kind: 0 | 1 | 2 | 3
  time: string
  level: string
  text: string
  source: string
  hasSource: boolean
}

export function levelMatches(want: string, level: string): boolean {
  if (!want) return true
  if (want === 'error') return level === 'error' || level === 'exception'
  return level === want
}

export function consoleRow(e: ConsoleEntry): ConsoleRow {
  const level = String(e.level ?? 'log')
  return {
    key: String(e.seq),
    kind: level === 'error' || level === 'exception' ? 2 : level === 'warning' ? 1 : level === 'debug' ? 3 : 0,
    time: clock(e.timestamp),
    level,
    text: String(e.text ?? ''),
    source: e.source ?? '',
    hasSource: !!e.source,
  }
}

/** Debounces a pattern field (300 ms) into the filter the feed uses; a bad regex keeps the last good one. */
export function usePattern(): {
  text: () => string
  typed: (text: string) => void
  filter: () => string
  bad: () => boolean
} {
  const [text, setText] = createSignal('')
  const [filter, setFilter] = createSignal('')
  const [bad, setBad] = createSignal(false)
  let timer: number | undefined
  onCleanup(() => clearTimeout(timer))
  return {
    text,
    filter,
    bad,
    typed: (value) => {
      setText(value)
      clearTimeout(timer)
      timer = setTimeout(() => {
        const p = value.trim()
        const ok = patternTest(p) !== null
        setBad(!ok)
        if (ok) setFilter(p)
      }, 300)
    },
  }
}

/** The long press on an entry. */
export async function copyMenu(title: string, label: string, text: string): Promise<void> {
  const choice = await iii.ui.actionMenu(null, { title, actions: [{ label, icon: 'copy' }] })
  if (choice !== 0) return
  try {
    await iii.clipboard.writeText(text)
    iii.ui.toast('Copied')
  } catch (err) {
    iii.ui.toast(readable(err), 'error')
  }
}

export function ConsolePage(props: { params: JsonObject }) {
  const session = String(props.params.session ?? '')
  const [level, setLevel] = createSignal(0)
  const pattern = usePattern()
  const want = () => LEVELS[level()]?.[1] ?? ''
  const feed = createFeed<ConsoleEntry, ConsoleRow>({
    session,
    trigger: TRIGGER.consoleEvent,
    filters: () => `${want()}|${pattern.filter()}`,
    read: (sinceSeq) =>
      browser.consoleRead(session, {
        level: want() || undefined,
        pattern: pattern.filter(),
        sinceSeq,
        limit: SEED_LIMIT,
      }),
    matches: (e) => levelMatches(want(), e.level) && (patternTest(pattern.filter())?.(e.text) ?? true),
    row: consoleRow,
  })
  const name = () => {
    const info = sessionOf(session)
    return info ? tabTitle(info.title, info.url) : null
  }
  const copy = (item: ConsoleRow) => void copyMenu(`${item.level} · ${item.time}`, 'Copy', item.text)
  const template = (tone: Tone) => (row: Bound<ConsoleRow>) => (
    <Column gap={2} padding={[8, 16]}>
      <Row gap={8}>
        <Text role="codeSmall" tone="faint" text={row.time} />
        <Text role="smallStrong" tone={tone} text={row.level} />
      </Row>
      <Text testTag="entry" role="code" maxLines={6} text={row.text} onLongPress={copy} />
      <Text role="idSmall" tone="faint" maxLines={1} overflow="middle" text={row.source} visible={row.hasSource} />
    </Column>
  )

  return (
    <Page
      testTag="console"
      title="Console"
      subtitle={name()}
      actions={<IconButton testTag="console-clear" icon="trash" label="Clear" onPress={feed.clear} />}
      top={
        <Column gap={8} padding={[8, 12]}>
          <Segmented
            testTag="console-level"
            compact
            options={LEVELS.map(([label]) => label)}
            selected={level()}
            onSelect={setLevel}
          />
          <TextField
            testTag="console-filter"
            value={pattern.text()}
            onChange={pattern.typed}
            placeholder="Filter (regex)"
            leadingIcon="search"
            error={pattern.bad() ? 'Not a valid pattern' : null}
            mono
            clearable
            autocorrect={false}
          />
          <Show when={feed.error()}>
            {(message: () => string) => (
              <StatusPanel
                testTag="console-error"
                variant="alert"
                headline="Couldn't read the console"
                detail={message()}
                action={
                  <Button testTag="console-retry" small onPress={feed.retry}>
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
        testTag="console-list"
        weight={1}
        dividers
        source={feed.rows}
        templates={[template('default'), template('warning'), template('danger'), template('faint')]}
        empty={
          <Show when={feed.loaded()} fallback={<Spinner padding={24} />}>
            <EmptyState
              testTag="console-empty"
              icon="terminal"
              title="No console messages"
              description="What the page logs shows up here."
            />
          </Show>
        }
      />
    </Page>
  )
}
