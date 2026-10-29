/** An action menu anchored to a button, and what the host answers. */
import { createSignal, defineApp, IconButton, iii, type NodeRef, Page, Text } from '@iii-dev/native-ui'

function Root() {
  const [picked, setPicked] = createSignal('Nothing picked')
  let more: NodeRef | undefined
  const actions = [
    { label: 'Copy URL', icon: 'copy' },
    { label: 'Zoom in', icon: 'zoom-in', hint: '110%' },
    { label: 'Close tab', icon: 'x', destructive: true },
  ]
  const open = async () => {
    const index = await iii.ui.actionMenu(more ?? null, { title: 'Example Domain', subtitle: 'example.com', actions })
    setPicked(index < 0 ? 'Dismissed' : `Picked ${actions[index].label}`)
  }
  return (
    <Page title="Tab" actions={<IconButton ref={more} icon="ellipsis-vertical" label="More" onPress={open} />}>
      <Text>{picked()}</Text>
    </Page>
  )
}

defineApp({ pages: { root: Root } })
