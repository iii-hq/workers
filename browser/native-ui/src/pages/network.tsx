/**
 * The network of a tab (design §8.4), its own card: failed only, a URL
 * pattern, and the requests (seed then live, newest 500). The worker keeps
 * no headers, bodies or timings: a request is its method, status, URL and
 * type. A long press copies the URL.
 */

import {
  type Bound,
  Button,
  Column,
  createSignal,
  EmptyState,
  IconButton,
  type JsonObject,
  List,
  Page,
  Row,
  Show,
  Spinner,
  StatusPanel,
  Switch,
  Text,
  TextField,
  type Tone,
} from '@iii-dev/native-ui'
import { tabTitle } from '../address'
import { browser, type NetworkEntry, TRIGGER } from '../api'
import { createFeed, patternTest, SEED_LIMIT } from '../feed'
import { sessionOf } from '../state'
import { copyMenu, usePattern } from './console'

export interface NetworkRow {
  key: string
  /** The template: 0 answered, 1 failed, 2 no answer yet, 3 an HTTP error status. */
  kind: 0 | 1 | 2 | 3
  method: string
  status: string
  url: string
  type: string
}

export function networkRow(e: NetworkEntry): NetworkRow {
  const status = typeof e.status === 'number' && e.status > 0 ? e.status : null
  return {
    key: String(e.seq),
    kind: e.failed ? 1 : status === null ? 2 : status >= 400 ? 3 : 0,
    method: String(e.method ?? 'GET').toUpperCase(),
    status: e.failed ? e.error || 'Failed' : status === null ? '…' : String(status),
    url: String(e.url ?? ''),
    type: (e.mime_type ?? '').split(';')[0].trim(),
  }
}

export function NetworkPage(props: { params: JsonObject }) {
  const session = String(props.params.session ?? '')
  const [failedOnly, setFailedOnly] = createSignal(false)
  const pattern = usePattern()
  const feed = createFeed<NetworkEntry, NetworkRow>({
    session,
    trigger: TRIGGER.networkEvent,
    filters: () => `${failedOnly()}|${pattern.filter()}`,
    read: (sinceSeq) =>
      browser.networkRead(session, {
        failedOnly: failedOnly(),
        pattern: pattern.filter(),
        sinceSeq,
        limit: SEED_LIMIT,
      }),
    matches: (e) => (!failedOnly() || e.failed) && (patternTest(pattern.filter())?.(e.url) ?? true),
    row: networkRow,
  })
  const name = () => {
    const info = sessionOf(session)
    return info ? tabTitle(info.title, info.url) : null
  }
  const copy = (item: NetworkRow) => void copyMenu(`${item.method} ${item.status}`, 'Copy URL', item.url)
  const template = (tone: Tone) => (row: Bound<NetworkRow>) => (
    <Column gap={2} padding={[8, 16]}>
      <Row gap={8}>
        <Text role="smallStrong" text={row.method} />
        <Text role="smallStrong" tone={tone} text={row.status} maxLines={1} />
        <Text role="small" tone="faint" weight={1} align="end" maxLines={1} text={row.type} />
      </Row>
      <Text testTag="request" role="codeSmall" maxLines={1} overflow="middle" text={row.url} onLongPress={copy} />
    </Column>
  )

  return (
    <Page
      testTag="network"
      title="Network"
      subtitle={name()}
      actions={<IconButton testTag="network-clear" icon="trash" label="Clear" onPress={feed.clear} />}
      top={
        <Column gap={8} padding={[8, 12]}>
          <Switch testTag="failed-only" label="Failed only" checked={failedOnly()} onChange={setFailedOnly} />
          <TextField
            testTag="network-filter"
            value={pattern.text()}
            onChange={pattern.typed}
            placeholder="Filter URLs (regex)"
            leadingIcon="search"
            error={pattern.bad() ? 'Not a valid pattern' : null}
            mono
            clearable
            autocorrect={false}
          />
          <Show when={feed.error()}>
            {(message: () => string) => (
              <StatusPanel
                testTag="network-error"
                variant="alert"
                headline="Couldn't read the network"
                detail={message()}
                action={
                  <Button testTag="network-retry" small onPress={feed.retry}>
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
        testTag="network-list"
        weight={1}
        dividers
        source={feed.rows}
        templates={[template('success'), template('danger'), template('faint'), template('warning')]}
        empty={
          <Show when={feed.loaded()} fallback={<Spinner padding={24} />}>
            <EmptyState
              testTag="network-empty"
              icon="network"
              title="No requests"
              description="What the page loads shows up here."
            />
          </Show>
        }
      />
    </Page>
  )
}
