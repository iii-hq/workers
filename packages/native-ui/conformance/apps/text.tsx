/** §5.7 "Texto": a TextField; the person types "ab" and the script normalizes it to "https://ab". */
import { createSignal, defineApp, TextField } from '@iii-dev/native-ui'

function Address() {
  const [url, setUrl] = createSignal('')
  return (
    <TextField
      value={url()}
      keyboard="url"
      imeAction="go"
      onChange={(text) => setUrl(text.startsWith('https://') ? text : `https://${text}`)}
    />
  )
}

defineApp({ pages: { root: Address } })
