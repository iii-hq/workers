/**
 * The text editor (design §6.6): a ticket's title (one line) or its
 * description (Markdown, several lines, mono). Save leaves through
 * `nav.pop({text})`; Back leaves without an answer, and the edit is dropped.
 */

import { Button, createSignal, type JsonObject, Page, Scroll, TextField, usePage } from '@iii-dev/native-ui'

export function TextPage(props: { params: JsonObject }) {
  const page = usePage()
  const multiline = props.params.multiline === true
  const initial = typeof props.params.initial === 'string' ? props.params.initial : ''
  const [text, setText] = createSignal(initial)
  const save = () => {
    if (!multiline && !text().trim()) return
    page.pop({ text: text() })
  }
  return (
    <Page
      testTag="text"
      title={typeof props.params.title === 'string' ? props.params.title : 'Edit'}
      bottom={
        <Button
          testTag="text-save"
          kind="primary"
          padding={[8, 12]}
          enabled={multiline || text().trim().length > 0}
          onPress={save}
        >
          Save
        </Button>
      }
    >
      <Scroll padding={[8, 12]}>
        <TextField
          testTag="text-field"
          value={text()}
          onChange={setText}
          lines={multiline ? 12 : 1}
          mono={multiline}
          autofocus
          selectAllOnFocus={!multiline}
          imeAction={multiline ? 'default' : 'done'}
          onSubmit={() => {
            if (!multiline) save()
          }}
          placeholder={multiline ? 'Markdown: what, why, and how to check it' : 'Title'}
        />
      </Scroll>
    </Page>
  )
}
