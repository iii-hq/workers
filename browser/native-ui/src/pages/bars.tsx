/**
 * The bars of an open tab (design §8.2-§8.4): the address bar (back,
 * forward, the address pill, reload, the page keyboard; tapping the pill
 * edits the URL) and the find bar that takes its place. One node holds
 * them so the page can move it between its top (wide) and bottom
 * (compact) slots without losing a half-typed address.
 */

import {
  Chip,
  Column,
  createSignal,
  IconButton,
  iii,
  onCleanup,
  Row,
  Show,
  Spinner,
  Text,
  TextField,
} from '@iii-dev/native-ui'
import { displayUrl, isBlank, isSecure, NOT_WEB, stripControls, webAddress } from '../address'
import { browser } from '../api'
import { readable } from '../format'
import { type HistoryAction, history, navigate, type OpenTab, sessionOf } from '../state'

export function Bars(props: { tab: OpenTab }) {
  const tab = props.tab
  const wide = () => iii.env.windowClass === 'wide'
  return (
    <Column testTag="bars" visible={!tab.closed() && (wide() || !tab.keyboard())}>
      <Show when={tab.find()} fallback={<AddressBar tab={tab} />}>
        <FindBar tab={tab} />
      </Show>
    </Column>
  )
}

/** A history button that turns into a spinner while its call is out. */
function HistoryButton(props: { tab: OpenTab; action: HistoryAction; icon: string; label: string }) {
  const tab = props.tab
  return (
    <Show
      when={tab.busy() !== props.action}
      fallback={<Spinner testTag={`${props.action}-busy`} size={18} padding={9} />}
    >
      <IconButton
        testTag={props.action}
        icon={props.icon}
        label={props.label}
        enabled={iii.env.online && tab.busy() === null}
        onPress={() => void history(tab, props.action)}
      />
    </Show>
  )
}

function AddressBar(props: { tab: OpenTab }) {
  const tab = props.tab
  const online = () => iii.env.online
  const readOnly = () => sessionOf(tab.id)?.read_only === true
  const [draft, setDraft] = createSignal(isBlank(tab.url()) ? '' : tab.url())
  let blur: number | undefined

  const stopBlur = () => {
    if (blur !== undefined) clearTimeout(blur)
    blur = undefined
  }
  onCleanup(stopBlur)

  const edit = () => {
    setDraft(isBlank(tab.url()) ? '' : tab.url())
    tab.setEditing(true)
  }
  const cancel = () => {
    stopBlur()
    tab.setEditing(false)
  }
  // D10: web addresses only. The console UI's rule adds the scheme; anything
  // that is not then http(s) with a host is refused here, before the worker.
  const go = (text: string) => {
    stopBlur()
    tab.setEditing(false)
    if (!stripControls(text).trim()) return
    const url = webAddress(text)
    if (url) void navigate(tab, url)
    else iii.ui.toast(NOT_WEB, 'error')
  }
  // Focus left the field (Back, a tap on the page): back to the pill, unless
  // the person was reaching for Go or Cancel (their press comes right after).
  const focus = (focused: boolean) => {
    stopBlur()
    if (!focused) blur = setTimeout(cancel, 250)
  }

  return (
    <Row testTag="address-bar" gap={4} padding={[6, 8]}>
      <Show
        when={tab.editing()}
        fallback={
          <>
            <HistoryButton tab={tab} action="back" icon="arrow-left" label="Back" />
            <HistoryButton tab={tab} action="forward" icon="arrow-right" label="Forward" />
            <Chip
              testTag="address"
              weight={1}
              icon={isSecure(tab.url()) ? 'lock' : null}
              text={displayUrl(tab.url()) || 'Type an address'}
              enabled={online()}
              onPress={edit}
            />
            <HistoryButton tab={tab} action="reload" icon="refresh-cw" label="Reload" />
            <IconButton
              testTag="keyboard"
              visible={!readOnly()}
              icon={tab.keyboard() ? 'keyboard-off' : 'keyboard'}
              label={tab.keyboard() ? 'Hide the keyboard' : 'Type into the page'}
              active={tab.keyboard()}
              onPress={() => tab.setKeyboard((open) => !open)}
            />
          </>
        }
      >
        <IconButton testTag="address-cancel" icon="x" label="Cancel" onPress={cancel} />
        <TextField
          testTag="address-field"
          weight={1}
          value={draft()}
          onChange={setDraft}
          onSubmit={go}
          onFocus={focus}
          placeholder="Type an address"
          keyboard="url"
          imeAction="go"
          autofocus
          selectAllOnFocus
        />
        <IconButton
          testTag="address-go"
          icon="arrow-right"
          label="Go"
          enabled={online() && draft().trim() !== ''}
          onPress={() => go(draft())}
        />
      </Show>
    </Row>
  )
}

function FindBar(props: { tab: OpenTab }) {
  const tab = props.tab
  const [query, setQuery] = createSignal(tab.find()?.query ?? '')
  let timer: number | undefined
  let asked = 0
  onCleanup(() => clearTimeout(timer))

  const run = async (q: string, action: 'search' | 'next' | 'previous') => {
    clearTimeout(timer)
    const ask = ++asked
    if (!q) {
      tab.setFind({ query: '', count: 0, index: 0 })
      return
    }
    try {
      const r = await browser.find(tab.id, q, action)
      if (ask === asked && tab.find()) tab.setFind({ query: q, count: r?.count ?? 0, index: r?.index ?? 0 })
    } catch (err) {
      iii.ui.toast(readable(err), 'error')
    }
  }
  const typed = (q: string) => {
    setQuery(q)
    clearTimeout(timer)
    timer = setTimeout(() => void run(q, 'search'), 300)
  }
  const close = () => {
    clearTimeout(timer)
    tab.setFind(null)
    browser.find(tab.id, '', 'close').catch(() => {})
  }
  const count = () => tab.find()?.count ?? 0
  const counter = () => {
    const f = tab.find()
    return f?.query ? `${f.index}/${f.count}` : ''
  }

  return (
    <Row testTag="find-bar" gap={4} padding={[6, 8]}>
      <TextField
        testTag="find-field"
        weight={1}
        value={query()}
        onChange={typed}
        onSubmit={(q) => void run(q, 'next')}
        placeholder="Find in page"
        leadingIcon="search"
        keyboard="search"
        imeAction="search"
        autofocus
      />
      <Text testTag="find-count" role="small" tone="muted">
        {counter()}
      </Text>
      <IconButton
        testTag="find-previous"
        icon="chevron-up"
        label="Previous match"
        enabled={count() > 0}
        onPress={() => void run(query(), 'previous')}
      />
      <IconButton
        testTag="find-next"
        icon="chevron-down"
        label="Next match"
        enabled={count() > 0}
        onPress={() => void run(query(), 'next')}
      />
      <IconButton testTag="find-close" icon="x" label="Close find" onPress={close} />
    </Row>
  )
}
