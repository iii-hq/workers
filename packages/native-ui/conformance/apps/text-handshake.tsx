/**
 * The edit-count handshake (design §5.6): the script lower-cases what the
 * person types, and a trigger fills the field. A write that reaches the host
 * after the person typed again carries an old count and is dropped; the
 * person's text wins and reaches the script as the next text entry.
 */
import { Column, createSignal, defineApp, Page, Text, TextField, useTrigger } from '@iii-dev/native-ui'

function Form() {
  const [name, setName] = createSignal('')
  const [submitted, setSubmitted] = createSignal('')
  useTrigger<{ text: string }>(
    'demo::fill',
    () => ({}),
    (payload) => setName(payload.text),
  )
  return (
    <Page title="Form">
      <Column gap={8} padding={16}>
        <TextField
          testTag="name"
          label="Name"
          value={name()}
          onChange={(text) => setName(text.toLowerCase())}
          onSubmit={(text) => setSubmitted(text)}
        />
        <Text testTag="echo">{submitted() ? `Sent ${submitted()}` : 'Nothing sent'}</Text>
      </Column>
    </Page>
  )
}

defineApp({ pages: { root: Form } })
