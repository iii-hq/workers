/**
 * Catalogue 2, named hues: an Icon, a Badge, a StatusDot and a Chip in each
 * hue (the prop by name, the index on the wire; none is the default and
 * does not travel), and a tap on the Chip changes its hue.
 */
import { Badge, Chip, Column, createSignal, defineApp, type Hue, Icon, Page, Row, StatusDot } from '@iii-dev/native-ui'

const HUES: Hue[] = ['none', 'blue', 'purple', 'teal', 'green', 'amber', 'rose']

function Root() {
  const [chip, setChip] = createSignal<Hue>('blue')
  return (
    <Page title="Hues">
      <Column gap={8} padding={16}>
        {HUES.map((hue) => (
          <Row testTag={`hue-${hue}`} gap={8}>
            <Icon name="circle" hue={hue} />
            <Badge hue={hue}>{hue}</Badge>
            <StatusDot hue={hue} />
          </Row>
        ))}
        <Chip testTag="chip" hue={chip()} onPress={() => setChip('green')}>
          Agent
        </Chip>
      </Column>
    </Page>
  )
}

defineApp({ catalog: 2, pages: { root: Root } })
