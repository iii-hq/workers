import type { Host } from '@iii-dev/console-ui'
import { Button, IconButton, Input, Panel, PanelBody, PanelHeader } from '@iii-dev/console-ui'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Check, ChevronDown, ChevronRight, Copy, ExternalLink } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useActions } from './hub'
import { deviceLabel, mmss, parseEnvironmentIds } from './model'
import { ErrorPanel, useNow } from './parts'
import type { Connection } from './types'

const DEFAULT_SECRET = 'IIICLOUD_API_KEY'

/** The code to type at the device page; the worker never puts it in a link. */
function DeviceCode({ connection }: { connection: Connection }) {
  const device = connection.device
  const now = useNow()
  const copy = useCopyFlash(device?.user_code ?? '')
  if (!device) return null
  const left = (Date.parse(device.expires_at) - now) / 1000
  const page = deviceLabel(device.verification_uri)
  return (
    <>
      <div className="iii-cloud-device">
        <div className="iii-cloud-device-text">
          <span>
            Open <span className="iii-cloud-mono">{page}</span>, type this code there, then approve.
          </span>
          <span className="iii-cloud-desc">
            Waiting for your approval · {left > 0 ? `the code expires in ${mmss(left)}` : 'the code expired'}
          </span>
        </div>
        <div className="iii-cloud-code">
          <span className="iii-cloud-code-value">{device.user_code}</span>
          <IconButton label={copy.state === 'copied' ? 'Copied' : 'Copy code'} variant="ghost" onClick={copy.copy}>
            {copy.state === 'copied' ? <Check /> : <Copy />}
          </IconButton>
        </div>
      </div>
      <div className="iii-cloud-row">
        <Button variant="secondary" size="sm" asChild>
          <a href={device.verification_uri} target="_blank" rel="noreferrer">
            Open {page}
            <ExternalLink />
          </a>
        </Button>
      </div>
    </>
  )
}

/** "Sign in to iii cloud" (Onb1): the browser sign-in and its device code; an API key behind a disclosure. */
export function ConnectView({ host, connection }: { host: Host; connection: Connection }) {
  const { busy, error, call } = useActions(host)
  const [secret, setSecret] = useState(connection.api_key?.secret ?? DEFAULT_SECRET)
  const [showKey, setShowKey] = useState(false)
  const [showIds, setShowIds] = useState(false)
  const [typed, setTyped] = useState('')
  const [ids, setIds] = useState('')
  const waiting = connection.status === 'waiting' && !!connection.device
  const blocked =
    error?.code === 'BROWSER_SIGN_IN_UNAVAILABLE'
      ? error
      : connection.error?.code === 'BROWSER_SIGN_IN_UNAVAILABLE'
        ? connection.error
        : null
  // The browser path is closed: the API key is the way forward, open and focused.
  const keyOpen = showKey || !!blocked
  const nameRef = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (blocked) nameRef.current?.focus()
  }, [!!blocked])
  const idsOk = ids.trim() === '' || parseEnvironmentIds(ids) !== null
  const shown = connection.error && connection.status !== 'waiting' && !blocked ? connection.error : null
  const browser = () => call('browser', 'iii-cloud::connect', { method: 'browser' })

  async function saveKey() {
    const stored = await call('key', 'secrets::set', { name: secret, value: typed, consumers: ['iii-cloud'] })
    if (stored === null) return
    setTyped('')
    await call('key', 'iii-cloud::connect', { method: 'api_key', secret, ...(parseEnvironmentIds(ids) ?? {}) })
  }

  return (
    <Panel aria-labelledby="iii-cloud-signin">
      <PanelHeader className="iii-cloud-panel-head">
        <span id="iii-cloud-signin">Sign in to iii cloud</span>
      </PanelHeader>
      <PanelBody className="iii-cloud-signin">
        {shown ? <ErrorPanel error={shown} /> : null}
        {blocked ? (
          <ErrorPanel
            error={blocked}
            variant="warn"
            action={
              <Button variant="secondary" size="sm" disabled={busy === 'browser'} onClick={browser}>
                Try again
              </Button>
            }
          />
        ) : waiting ? (
          <DeviceCode connection={connection} />
        ) : (
          <>
            <p className="iii-cloud-setup-text">
              You sign in with your iii cloud account in the browser. The worker keeps the session on this machine; your
              agent can use it but never reads it.
            </p>
            <div className="iii-cloud-row">
              <Button variant="primary" disabled={busy === 'browser'} onClick={browser}>
                Sign in with the browser
              </Button>
              <span className="iii-cloud-desc">No account yet? The same page creates one.</span>
            </div>
          </>
        )}
        <div className="iii-cloud-signin-key">
          <Button
            variant="ghost"
            size="sm"
            className="iii-cloud-disclose"
            aria-expanded={keyOpen}
            onClick={() => setShowKey(!keyOpen)}
          >
            {keyOpen ? <ChevronDown /> : <ChevronRight />}
            Can't use a browser here? Use an API key
          </Button>
          {keyOpen ? (
            <div className="iii-cloud-signin-form">
              <p className="iii-cloud-desc">
                An API key can check and deploy one environment. It can't create projects or environments, so setup
                needs the browser at least once.
              </p>
              <label className="iii-cloud-lbl" htmlFor="iii-cloud-secret-name">
                Secret name
              </label>
              <Input
                ref={nameRef}
                id="iii-cloud-secret-name"
                className="iii-cloud-mono"
                value={secret}
                onChange={(v) => setSecret(v.toUpperCase().replace(/[^A-Z0-9_]/g, ''))}
              />
              <span className="iii-cloud-lbl" aria-hidden>
                API key
              </span>
              <Input type="password" aria-label="API key" autoComplete="off" value={typed} onChange={setTyped} />
              <p className="iii-cloud-desc">The key stays in the harness secrets vault, never in the chat.</p>
              <Button
                variant="ghost"
                size="sm"
                className="iii-cloud-disclose"
                aria-expanded={showIds}
                onClick={() => setShowIds(!showIds)}
              >
                {showIds ? <ChevronDown /> : <ChevronRight />}
                Environment ids, if the key cannot be resolved
              </Button>
              {showIds ? (
                <>
                  <label className="iii-cloud-lbl" htmlFor="iii-cloud-env-ids">
                    Environment
                  </label>
                  <Input
                    id="iii-cloud-env-ids"
                    className="iii-cloud-mono"
                    placeholder="<team_id>/<environment_id>"
                    aria-invalid={!idsOk}
                    value={ids}
                    onChange={setIds}
                  />
                </>
              ) : null}
              <div className="iii-cloud-row">
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={!typed || !secret || !idsOk || busy === 'key'}
                  onClick={saveKey}
                >
                  Save key
                </Button>
              </div>
            </div>
          ) : null}
        </div>
        {error && !blocked ? <ErrorPanel error={error} /> : null}
      </PanelBody>
    </Panel>
  )
}
