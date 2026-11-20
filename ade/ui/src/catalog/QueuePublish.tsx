/**
 * Publish a message onto a queue subscriber's topic.
 *
 * Unlike calling the subscriber's target directly, this is the real path:
 * `iii::durable::publish` puts the message on the queue, the queue worker
 * calls `queue::on-message`, and the subscriber's retry and DLQ behavior
 * applies exactly as it would in production. That is also why it asks before sending.
 */

import { Button, CodeEditor, type Host, JsonHighlight, StatusPanel } from '@iii-dev/console-ui'
import { useEffect, useRef, useState } from 'react'
import { type InvokeOutcome, invoke } from './engine'
import { pretty } from './schema'

export function QueuePublish({ host, topic }: { host: Host; topic: string }) {
  const [body, setBody] = useState('{}')
  const [confirming, setConfirming] = useState(false)
  const [sending, setSending] = useState(false)
  const [outcome, setOutcome] = useState<InvokeOutcome | null>(null)
  const [invalid, setInvalid] = useState<string | null>(null)

  // Guards an in-flight publish across a topic switch: the stale result must
  // not land on the newly selected topic's fresh form.
  const activeTopic = useRef(topic)
  activeTopic.current = topic

  useEffect(() => {
    setBody('{}')
    setOutcome(null)
    setInvalid(null)
    setConfirming(false)
    setSending(false)
  }, [topic])

  const publish = async () => {
    let data: unknown
    try {
      data = JSON.parse(body)
    } catch (err) {
      setInvalid(err instanceof Error ? err.message : 'Invalid JSON')
      return
    }
    setInvalid(null)
    setConfirming(false)
    setSending(true)
    const sentTopic = topic
    const result = await invoke(host, 'iii::durable::publish', { topic, data })
    if (activeTopic.current !== sentTopic) return
    setOutcome(result)
    setSending(false)
  }

  return (
    <div className="console-catalog-invoke">
      <div className="console-catalog-note">
        Publishes to <code>{topic}</code> through the queue, so every consumer of this topic receives it and the
        subscriber's retry and dead-letter behavior applies.
      </div>
      <CodeEditor
        fontSize={13}
        value={body}
        onChange={setBody}
        language="json"
        className="console-catalog-editor"
        aria-label={`message body for ${topic}`}
      />
      <div className="console-catalog-invoke-foot">
        {confirming ? (
          <>
            <Button className="console-catalog-btn" variant="primary" size="sm" onClick={publish} disabled={sending}>
              {sending ? 'Publishing…' : 'Publish to the queue'}
            </Button>
            <Button
              className="console-catalog-btn"
              variant="ghost"
              size="sm"
              onClick={() => setConfirming(false)}
              disabled={sending}
            >
              Cancel
            </Button>
          </>
        ) : (
          <Button
            className="console-catalog-btn"
            variant="primary"
            size="sm"
            onClick={() => setConfirming(true)}
            disabled={sending}
          >
            Publish message
          </Button>
        )}
        {invalid ? <span className="console-catalog-invalid">{invalid}</span> : null}
        {outcome ? (
          <span className={outcome.ok ? 'console-catalog-ok' : 'console-catalog-invalid'}>
            {outcome.ok ? 'Published' : 'Failed'} · {Math.round(outcome.durationMs)} ms
          </span>
        ) : null}
      </div>
      {outcome?.error ? <StatusPanel variant="alert" headline={outcome.error} /> : null}
      {outcome?.ok && outcome.data !== null && outcome.data !== undefined ? (
        <JsonHighlight code={pretty(outcome.data)} className="console-catalog-result" wrap />
      ) : null}
      <p className="console-catalog-fine">
        Publishes for real, through <code>iii::durable::publish</code>.
      </p>
    </div>
  )
}
