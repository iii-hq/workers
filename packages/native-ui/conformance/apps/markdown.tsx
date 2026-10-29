/**
 * Catalogue 2, Markdown: a dense Markdown with a web link and an app link
 * (`kan:KAN-12`); the host sends each tap as `link [href]` and opens
 * nothing, and the script shows the last one.
 */
import { Column, createSignal, defineApp, Markdown, Page, Text } from '@iii-dev/native-ui'

const TEXT = `## Release notes

- Faster boards
- See [the docs](https://example.com) and [KAN-12](kan:KAN-12)

\`\`\`
pnpm test
\`\`\``

function Root() {
  const [link, setLink] = createSignal('')
  return (
    <Page title="Notes">
      <Column gap={8} padding={16}>
        <Markdown testTag="notes" dense selectable onLink={setLink}>
          {TEXT}
        </Markdown>
        <Text testTag="last-link">{link()}</Text>
      </Column>
    </Page>
  )
}

defineApp({ catalog: 2, pages: { root: Root } })
