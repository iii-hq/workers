import { Button, Input, Select } from '@iii-dev/console-ui'
import { useState } from 'react'
import type { ComputerDisplay, StartSessionInput } from '../lib/computer'

/**
 * Start control: the three ways to get a desktop, as one choice rather than
 * three optional fields. `native` drives this machine (with a display picker
 * when the host reports more than one), `sandbox` boots a desktop image in a
 * microVM, `remote` connects to a desktop somebody else booted.
 */

type Mode = 'native' | 'sandbox' | 'remote'

const MODE_OPTIONS = [
  { value: 'native' as const, label: 'this machine' },
  { value: 'sandbox' as const, label: 'sandbox image' },
  { value: 'remote' as const, label: 'remote endpoint' },
]

interface StartSessionFormProps {
  displays: ComputerDisplay[]
  starting: boolean
  onStart: (input: StartSessionInput) => void
}

export function StartSessionForm({
  displays,
  starting,
  onStart,
}: StartSessionFormProps) {
  const [mode, setMode] = useState<Mode>('native')
  const [image, setImage] = useState('')
  const [endpoint, setEndpoint] = useState('')
  const [monitor, setMonitor] = useState<string | undefined>(undefined)

  const submit = () => {
    if (starting) return
    if (mode === 'sandbox') {
      onStart({ image: image.trim() || 'desktop' })
      return
    }
    if (mode === 'remote') {
      const trimmed = endpoint.trim()
      if (!trimmed) return
      onStart({ endpoint: trimmed })
      return
    }
    onStart(monitor != null ? { monitor: Number(monitor) } : {})
  }

  return (
    <form
      className="cp-ui-start"
      onSubmit={(e) => {
        e.preventDefault()
        submit()
      }}
    >
      <Select<Mode>
        value={mode}
        options={MODE_OPTIONS}
        onChange={setMode}
        aria-label="what to drive"
        className="cp-ui-start-mode"
      />
      {mode === 'sandbox' ? (
        <Input
          value={image}
          onChange={setImage}
          placeholder="image (default: desktop)"
          preserveCase
          aria-label="sandbox image"
          className="cp-ui-start-field"
        />
      ) : null}
      {mode === 'remote' ? (
        <Input
          value={endpoint}
          onChange={setEndpoint}
          placeholder="ws://host:8000"
          preserveCase
          aria-label="endpoint"
          className="cp-ui-start-field"
        />
      ) : null}
      {mode === 'native' && displays.length > 1 ? (
        <Select
          value={monitor}
          options={displays.map((d) => ({
            value: String(d.index),
            label: `${d.name || `display ${d.index}`}${d.primary ? ' (primary)' : ''}`,
          }))}
          onChange={setMonitor}
          allowEmpty
          emptyLabel="display under cursor"
          onClear={() => setMonitor(undefined)}
          placeholder="display under cursor"
          aria-label="display"
          className="cp-ui-start-field"
        />
      ) : null}
      <Button
        type="submit"
        variant="primary"
        size="sm"
        disabled={starting || (mode === 'remote' && endpoint.trim() === '')}
      >
        {starting ? 'starting...' : 'start session'}
      </Button>
    </form>
  )
}
