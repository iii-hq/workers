import type { Host } from '@iii-dev/console-ui'
import { Button, Input } from '@iii-dev/console-ui'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { type ReactNode, useId, useState } from 'react'
import { useActions } from './hub'
import { envName, envPageUrl } from './model'
import { ErrorPanel, Mono } from './parts'
import type { SecretRequest, Status } from './types'

/** Where the value goes, in the person's words ("Goes to…" until it is saved, "It went to…" after). */
export function Destinations({ status, saved }: { status: Status; saved: boolean }) {
  const env = envName(status)
  const url = envPageUrl(status, 'config')
  const lead = saved ? 'Saved. It went to' : 'Goes to'
  if (status.connection.method === 'api_key') {
    return (
      <span className="iii-cloud-psmall">
        {lead} the .env of the agent's copy of your code for the local run.{' '}
        {url ? (
          <a href={url} target="_blank" rel="noreferrer">
            Set it in iii cloud's console too
          </a>
        ) : (
          "Set it in iii cloud's console too"
        )}
        : an API key can't write variables.
      </span>
    )
  }
  return (
    <span className="iii-cloud-psmall">
      {lead} the .env of the agent's copy of your code for the local run and to iii cloud{env ? ` for ${env}` : ''}.
    </span>
  )
}

/**
 * One secret, typed in place: the name, why it is needed, a password field and Save.
 * The value goes only to `iii-cloud::secret::provide`; without an open request the
 * page opens one first. It never reaches the chat.
 */
export function SecretEntry({
  host,
  status,
  name,
  why,
  request,
  extra,
  onSaved,
}: {
  host: Host
  status: Status
  name: string
  why: string
  request?: SecretRequest
  extra?: ReactNode
  /** After the value reached `secret::provide` (the field is empty again). */
  onSaved?: () => void
}) {
  const { busy, error, call } = useActions(host)
  const [value, setValue] = useState('')
  const [saved, setSaved] = useState(false)
  const [whyOpen, setWhyOpen] = useState(false)
  // One name can be typed in two places at once (the agent's ask and the Settings row).
  const id = useId()
  const save = async () => {
    let requestId = request?.id
    if (!requestId) {
      const made = await call<SecretRequest>('provide', 'iii-cloud::secret', { name, description: why })
      requestId = made?.id
      if (!requestId) return
    }
    const out = await call('provide', 'iii-cloud::secret::provide', { request_id: requestId, value })
    if (out !== null) {
      setValue('')
      setSaved(true)
      onSaved?.()
    }
  }
  return (
    <div className="iii-cloud-secret">
      <label className="iii-cloud-lbl" htmlFor={id}>
        <Mono>{name}</Mono>
      </label>
      <button
        type="button"
        className="iii-cloud-why"
        data-open={whyOpen ? '' : undefined}
        aria-expanded={whyOpen}
        onClick={() => setWhyOpen(!whyOpen)}
      >
        {whyOpen ? <ChevronDown /> : <ChevronRight />}
        <span className="iii-cloud-why-text">{why}</span>
      </button>
      <div className="iii-cloud-row">
        <div className="iii-cloud-grow">
          <Input id={id} type="password" autoComplete="off" value={value} onChange={setValue} />
        </div>
        <Button variant="primary" disabled={!value || busy === 'provide'} onClick={save}>
          Save
        </Button>
        {extra}
      </div>
      <Destinations status={status} saved={saved} />
      {error ? <ErrorPanel error={error} /> : null}
    </div>
  )
}
