/**
 * Catalogue 3, a toast with an action: Archive shows "Archived" with Undo
 * (a request with an answer, not a notification); the host answers that the
 * person tapped Undo, and the script puts the item back.
 */
import { Button, Column, createSignal, defineApp, iii, Page, Text } from '@iii-dev/native-ui'

function Root() {
  const [state, setState] = createSignal('inbox')
  const archive = async () => {
    setState('archived')
    const undone = await iii.ui.toast('Archived', { kind: 'success', action: 'Undo' })
    if (undone) setState('inbox')
  }
  return (
    <Page title="Mail">
      <Column gap={8} padding={16}>
        <Text testTag="state">{state()}</Text>
        <Button testTag="archive" onPress={archive}>
          Archive
        </Button>
      </Column>
    </Page>
  )
}

defineApp({ catalog: 3, pages: { root: Root } })
