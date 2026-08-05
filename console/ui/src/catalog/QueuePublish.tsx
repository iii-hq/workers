/**
 * Publish a message onto a queue subscriber's topic.
 *
 * Unlike calling the subscriber's function directly, this is the real path:
 * `iii::durable::publish` puts the message on the queue, the queue worker
 * delivers it, and the binding's retry and DLQ behavior applies exactly as it
 * would in production. That is also why it asks before sending.
 */

import {
  Button,
  CodeEditor,
  type Host,
  JsonHighlight,
} from '@iii-dev/console-ui'
import { useEffect, useState } from 'react'
import { type InvokeOutcome, invoke } from './engine'
import { pretty } from './schema'
import { Chip } from './widgets'

export function QueuePublish({ host, topic }: { host: Host; topic: string }) {
  const [body, setBody] = useState('{}')
  const [confirming, setConfirming] = useState(false)
  const [sending, setSending] = useState(false)
  const [outcome, setOutcome] = useState<InvokeOutcome | null>(null)
  const [invalid, setInvalid] = useState<string | null>(null)

  useEffect(() => {
    setBody('{}')
    setOutcome(null)
    setInvalid(null)
    setConfirming(false)
  }, [topic])

  const publish = async () => {
    let data: unknown
    try {
      data = JSON.parse(body)
    } catch (err) {
      setInvalid(err instanceof Error ? err.message : 'invalid JSON')
      return
    }
    setInvalid(null)
    setConfirming(false)
    setSending(true)
    setOutcome(await invoke(host, 'iii::durable::publish', { topic, data }))
    setSending(false)
  }

  return (
    <div className="console-catalog-invoke">
      <div className="console-catalog-note">
        publishes to <code>{topic}</code> through the queue, so every consumer
        of this topic receives it and the binding's retry and dead-letter
        behavior applies.
      </div>
      <CodeEditor
        value={body}
        onChange={setBody}
        language="json"
        className="console-catalog-editor"
        aria-label={`message body for ${topic}`}
      />
      <div className="console-catalog-invoke-foot">
        {confirming ? (
          <>
            <Button size="sm" onClick={publish} disabled={sending}>
              {sending ? 'publishing…' : 'yes, publish'}
            </Button>
            <Button
              variant="pill"
              size="sm"
              onClick={() => setConfirming(false)}
              disabled={sending}
            >
              cancel
            </Button>
          </>
        ) : (
          <Button
            size="sm"
            onClick={() => setConfirming(true)}
            disabled={sending}
          >
            publish message
          </Button>
        )}
        {invalid ? (
          <span className="console-catalog-invalid">{invalid}</span>
        ) : null}
        {outcome ? (
          <span
            className={
              outcome.ok ? 'console-catalog-ok' : 'console-catalog-invalid'
            }
          >
            {outcome.ok ? 'published' : 'failed'} ·{' '}
            {Math.round(outcome.durationMs)}ms
          </span>
        ) : null}
      </div>
      {outcome?.error ? (
        <div className="console-catalog-error">{outcome.error}</div>
      ) : null}
      {outcome?.ok && outcome.data !== null && outcome.data !== undefined ? (
        <JsonHighlight
          code={pretty(outcome.data)}
          className="console-catalog-result"
          wrap
        />
      ) : null}
      <Chip k="fires" v="the real queue, through iii::durable::publish" />
    </div>
  )
}
