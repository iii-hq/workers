/** Tree ops: conditional subtrees (destroy), a node kept and attached again, reorders (move), slots, optimistic inputs. */
import {
  Button,
  Column,
  createSignal,
  defineApp,
  For,
  type JSX,
  Page,
  Segmented,
  Sheet,
  Show,
  Switch,
  Text,
} from '@iii-dev/native-ui'

function Root() {
  const [open, setOpen] = createSignal(true)
  const [order, setOrder] = createSignal(['a', 'b', 'c'])
  const [checked, setChecked] = createSignal(false)
  const [accept, setAccept] = createSignal(true)
  const [tab, setTab] = createSignal(0)
  const [sheet, setSheet] = createSignal(false)
  const [kept, setKept] = createSignal(true)
  // A node held outside the JSX that shows it: destroyed when hidden, sent again under a new id when shown.
  const badge: JSX.Element = <Text testTag="kept">Kept</Text>
  return (
    <Page
      title="Tree"
      actions={<Button testTag="sheet" text="Sheet" onPress={() => setSheet(true)} />}
      bottom={<Text testTag="bottom">Bottom</Text>}
    >
      <Column testTag="col" gap={4}>
        <Show when={open()} fallback={<Text testTag="closed">Closed</Text>}>
          <Column testTag="details">
            <Text>Details</Text>
            <Text>More</Text>
          </Column>
        </Show>
        <For each={order()}>{(name) => <Text testTag={`item-${name}`}>{name}</Text>}</For>
        <Show when={kept()}>{badge}</Show>
        loose text
      </Column>
      <Button testTag="toggle" text="Toggle" onPress={() => setOpen(!open())} />
      <Button testTag="rotate" text="Rotate" onPress={() => setOrder([...order().slice(1), order()[0]])} />
      <Button testTag="reverse" text="Reverse" onPress={() => setOrder([...order()].reverse())} />
      <Button testTag="kept-toggle" text="Kept" onPress={() => setKept(!kept())} />
      <Button testTag="strict" text="Strict" onPress={() => setAccept(!accept())} />
      <Switch testTag="switch" label="Wi-Fi" checked={checked()} onChange={(v) => accept() && setChecked(v)} />
      <Segmented testTag="seg" options={['All', 'Errors']} selected={tab()} onSelect={(i) => accept() && setTab(i)} />
      <Sheet testTag="the-sheet" open={sheet()} title="Sheet" onDismiss={() => setSheet(false)}>
        <Text>In the sheet</Text>
      </Sheet>
    </Page>
  )
}

defineApp({ pages: { root: Root } })
