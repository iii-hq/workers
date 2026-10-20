import { Button, type Host, Input, StatusPanel, uiClasses } from '@iii-dev/console-ui'
import { useCopyFlash } from '@iii-dev/console-ui/hooks'
import { Check, ChevronRight, Copy } from 'lucide-react'
import { useCallback, useId, useState } from 'react'
import type { Condition } from './conditions'
import type { Step, StepRecord, StepState, TourController } from './use-tour'

/**
 * One step's content, the same in the card and in the page: what to read,
 * the one thing to do, and — for a step that waits on a trigger — what it
 * is waiting for. The caller supplies the frame around it.
 */
export function StepContent({ host, tour, step }: { host: Host; tour: TourController; step: Step }) {
  const state = tour.stateOf(step)
  const record = tour.records[step.id]
  const reading = tour.open === step.id
  // Opened by the operator only — a trigger that fires does not throw its
  // payload open over what is being read.
  const [conditionOpen, setConditionOpen] = useState(false)
  const waitingForScreen = reading && Boolean(step.to_open) && tour.awaitingScreen && state !== 'complete'
  // Open, but in a tab that is not showing: say so, and the spotlight is on
  // that tab. The step waits for it to be the one on screen.
  const screenElsewhere = reading && Boolean(step.to_open) && tour.screenElsewhere && state !== 'complete'
  return (
    <>
      <p className="learn-body">{step.body}</p>
      {reading && step.on_closed && tour.closedNote ? (
        <p className="learn-note" role="status">
          {step.on_closed.body}
        </p>
      ) : null}
      {waitingForScreen ? (
        <p className="learn-note learn-note--todo" role="status">
          {step.to_open?.body}
        </p>
      ) : null}
      {screenElsewhere ? (
        <p className="learn-note learn-note--todo" role="status">
          {step.to_open?.elsewhere ?? 'It is open in another tab. Switch to that tab to see it.'}
        </p>
      ) : null}
      {step.condition?.prompt && state !== 'complete' ? (
        <Copyable label="Or ask the agent" text={step.condition.prompt} />
      ) : null}
      {step.id === 'stay-in-touch' ? (
        <StayInTouch
          host={host}
          subscribed={tour.subscribed}
          onSubscribed={() => {
            tour.markSubscribed()
            if (state !== 'complete') tour.complete(step.id)
          }}
          onContinue={() => (state === 'complete' ? tour.setOpen(tour.active?.id ?? null) : tour.complete(step.id))}
        />
      ) : null}
      {step.ask && state !== 'complete' ? (
        <div className="learn-ask">
          <div className="learn-actions">
            <Button className="learn-action" onClick={() => tour.ask(step)}>
              {step.ask.label}
            </Button>
          </div>
          <Prompt text={step.ask.text} />
        </div>
      ) : null}
      {state !== 'complete' && !step.condition && !step.ask && step.id !== 'stay-in-touch' ? (
        <div className="learn-actions">
          {waitingForScreen ? (
            // The button opens the screen beside the chat; the step moves on
            // once it is there. Skipping is there so it is never a dead end.
            <>
              <Button className="learn-action" onClick={() => tour.openScreen(step)}>
                {step.to_open?.label}
              </Button>
              <Button variant="ghost" className="learn-ghost" onClick={() => tour.complete(step.id)}>
                Skip this step
              </Button>
            </>
          ) : screenElsewhere ? (
            // The operator brings the tab up; the step moves on when they have.
            <Button variant="ghost" className="learn-ghost" onClick={() => tour.complete(step.id)}>
              Skip this step
            </Button>
          ) : (
            <Button className="learn-action" onClick={() => tour.complete(step.id)}>
              {step.to_open ? 'Continue' : 'Got it'}
            </Button>
          )}
        </div>
      ) : null}
      {step.condition ? (
        <ConditionRow
          condition={step.condition}
          fired={record?.fired ?? null}
          open={conditionOpen}
          onToggle={() => setConditionOpen((value) => !value)}
        />
      ) : null}
    </>
  )
}

/**
 * What a step's button sends, shown as the message it is: a label, then the
 * text. The operator reads it before pressing, and never has to type it.
 */
function Prompt({ text }: { text: string }) {
  return (
    <div className="learn-prompt">
      <span className="learn-prompt-head">
        <span className="learn-prompt-label">Message to the agent</span>
      </span>
      <p className="learn-prompt-text">{text}</p>
    </div>
  )
}

/** The row's trailing state, as the dialog's chips: a dot for a live one. */
export function StateChip({ state, step, record }: { state: StepState; step: Step; record: StepRecord | undefined }) {
  if (state === 'pending') return null
  if (state === 'complete') {
    return (
      <span className="learn-chip" data-tone="ok">
        <Check className="learn-chip-mark" strokeWidth={2.5} aria-hidden="true" />
        {record ? `Done ${when(record.at)}` : 'Done'}
      </span>
    )
  }
  return (
    <span className="learn-chip" data-tone={step.condition ? 'live' : 'now'}>
      {step.condition ? <span className={`learn-chip-dot ${uiClasses.pulse}`} aria-hidden="true" /> : null}
      {step.condition ? 'Waiting' : 'Now'}
    </span>
  )
}

/**
 * What the step is waiting on, as one line at the foot of its panel: the
 * state, the trigger, and — once it fires — when. Expanding it shows the
 * binding and the payload the engine delivered, the way the harness shows a
 * function call. It opens when the operator opens it: a trigger that fires
 * does not throw its payload over whatever is being read.
 */
function ConditionRow({
  condition,
  fired,
  open,
  onToggle,
}: {
  condition: Condition
  fired: StepRecord['fired']
  open: boolean
  onToggle: () => void
}) {
  const hasConfig = Object.keys(condition.config ?? {}).length > 0
  return (
    <div className="learn-condition" data-fired={fired ? 'true' : undefined}>
      <button type="button" className="learn-disclosure" aria-expanded={open} onClick={onToggle}>
        <ChevronRight className={`learn-caret ${uiClasses.icon}`} data-open={open || undefined} aria-hidden="true" />
        <span className="learn-disclosure-label">Trigger</span>
        <span className="learn-disclosure-summary">
          {fired ? (
            <>
              <Check className="learn-chip-mark" strokeWidth={2.5} aria-hidden="true" />
              Fired {when(fired.at)}
            </>
          ) : (
            <>
              <span className={`learn-chip-dot ${uiClasses.pulse}`} aria-hidden="true" />
              {condition.label}
            </>
          )}
        </span>
      </button>
      <div className="learn-panel learn-panel--inner" data-open={open || undefined} inert={!open}>
        <div className="learn-panel-clip">
          <div className="learn-panel-body learn-terminal">
            <span className="learn-terminal-line">
              <span className="learn-terminal-prompt">$</span> {fired?.trigger_type ?? condition.type}
              {hasConfig ? <span className="learn-terminal-dim"> {JSON.stringify(condition.config)}</span> : null}
            </span>
            {fired ? (
              <pre className="learn-pre">{format(fired.payload)}</pre>
            ) : (
              <span className="learn-terminal-dim"># {condition.hint ?? condition.label}</span>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

/** The product-update links. Kept here and not in the tour content: they are
    chrome for one step, not text. */
const SOCIALS: { label: string; href: string; path: string }[] = [
  {
    label: 'Discord',
    href: 'https://discord.gg/iiidev',
    path: 'M20.317 4.37a19.79 19.79 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028 14.09 14.09 0 0 0 1.226-1.994.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.099.245.197.372.292a.077.077 0 0 1-.006.128 12.299 12.299 0 0 1-1.873.891.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.84 19.84 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.03zM8.02 15.331c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.956 2.418-2.157 2.418zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.955-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.946 2.418-2.157 2.418z',
  },
  {
    label: 'GitHub',
    href: 'https://github.com/iii-hq/iii',
    path: 'M12 0C5.37 0 0 5.37 0 12a12 12 0 0 0 8.205 11.385c.6.105.825-.255.825-.57 0-.285-.015-1.23-.015-2.235-3.015.555-3.795-.735-4.035-1.41-.135-.345-.72-1.41-1.23-1.695-.42-.225-1.02-.78-.015-.795.945-.015 1.62.87 1.845 1.23 1.08 1.815 2.805 1.305 3.495.99.105-.78.42-1.305.765-1.605-2.67-.3-5.46-1.335-5.46-5.925 0-1.305.465-2.385 1.23-3.225-.12-.3-.54-1.53.12-3.18 0 0 1.005-.315 3.3 1.23a9.6 9.6 0 0 1 3-.405c1.02 0 2.04.135 3 .405 2.295-1.56 3.3-1.23 3.3-1.23.66 1.65.24 2.88.12 3.18.765.84 1.23 1.905 1.23 3.225 0 4.605-2.805 5.625-5.475 5.925.435.375.81 1.095.81 2.22 0 1.605-.015 2.895-.015 3.3 0 .315.225.69.825.57A12.005 12.005 0 0 0 24 12c0-6.63-5.37-12-12-12z',
  },
  {
    label: 'X',
    href: 'https://x.com/iiidevs',
    path: 'M18.901 1.153h3.68l-8.04 9.19L24 22.846h-7.406l-5.8-7.584-6.638 7.584H.474l8.6-9.83L0 1.154h7.594l5.243 6.932ZM17.61 20.644h2.039L6.486 3.24H4.298Z',
  },
  {
    label: 'LinkedIn',
    href: 'https://www.linkedin.com/company/iii-dev',
    path: 'M20.447 20.452h-3.554v-5.569c0-1.328-.027-3.037-1.852-3.037-1.853 0-2.136 1.445-2.136 2.939v5.667H9.351V9h3.414v1.561h.046c.477-.9 1.637-1.85 3.37-1.85 3.601 0 4.267 2.37 4.267 5.455v6.286zM5.337 7.433a2.062 2.062 0 0 1-2.063-2.065 2.064 2.064 0 1 1 2.063 2.065zm1.782 13.019H3.555V9h3.564v11.452zM22.225 0H1.771C.792 0 0 .774 0 1.729v20.542C0 23.227.792 24 1.771 24h20.451C23.2 24 24 23.227 24 22.271V1.729C24 .774 23.2 0 22.225 0z',
  },
]

/**
 * The signup box, then the same places as links. The worker does the POST —
 * the page only carries the address the operator typed.
 */
function StayInTouch({
  host,
  subscribed,
  onSubscribed,
  onContinue,
}: {
  host: Host
  subscribed: boolean
  onSubscribed: () => void
  /** Skip the signup, or go on from it: closes the step when it is still open. */
  onContinue: () => void
}) {
  const fieldId = useId()
  const [email, setEmail] = useState('')
  const [status, setStatus] = useState<'idle' | 'sending' | 'done' | 'failed'>('idle')
  const [message, setMessage] = useState('')

  const submit = useCallback(
    (event: React.FormEvent) => {
      event.preventDefault()
      if (!email.trim() || status === 'sending') return
      setStatus('sending')
      void host.iii
        .trigger('learn-iii::subscribe', { email: email.trim(), source: 'learn_iii_flow' })
        .then(() => {
          setStatus('done')
          onSubscribed()
        })
        .catch((error: unknown) => {
          setStatus('failed')
          setMessage(
            error instanceof Error ? error.message : 'Unable to sign you up. Check your connection and try again.',
          )
        })
    },
    [email, host, status, onSubscribed],
  )

  return (
    <div className="learn-stack">
      {/* One inset panel for the signup, so the field and its two actions
          read as one thing; the confirmation takes its place in the same
          panel, so the card does not jump. */}
      <div className="learn-signup">
        {subscribed || status === 'done' ? (
          // Already on the list (an earlier visit, a restarted tour): the
          // step still needs a way on.
          <div className="learn-form-actions">
            <p className="learn-signup-done" role="status">
              <Check className={uiClasses.icon} strokeWidth={2.5} aria-hidden="true" />
              You are on the list.
            </p>
            <Button type="button" className="learn-action" onClick={onContinue}>
              Continue
            </Button>
          </div>
        ) : (
          <form className="learn-form" onSubmit={submit}>
            <label className="learn-signup-label" htmlFor={fieldId}>
              Email for product updates
            </label>
            <Input
              id={fieldId}
              type="email"
              required
              value={email}
              onChange={setEmail}
              placeholder="you@example.com"
              className="learn-field"
            />
            <div className="learn-form-actions">
              <Button
                type="button"
                variant="ghost"
                className="learn-ghost"
                onClick={onContinue}
                disabled={status === 'sending'}
              >
                Skip this step
              </Button>
              <Button type="submit" className="learn-action" disabled={status === 'sending'}>
                {status === 'sending' ? 'Sending…' : 'Get updates'}
              </Button>
            </div>
          </form>
        )}
        {status === 'failed' ? <StatusPanel variant="alert" headline={message} role="alert" /> : null}
      </div>
      <div className="learn-follow">
        <span className="learn-follow-label">Follow along</span>
        <div className="learn-links">
          {SOCIALS.map((social) => (
            <Button key={social.label} asChild variant="icon" size="icon" className="learn-link">
              <a href={social.href} target="_blank" rel="noreferrer" aria-label={social.label} title={social.label}>
                {/* Brand marks lucide does not ship. lint-allow no-inline-svg */}
                <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" className={uiClasses.icon}>
                  <path d={social.path} />
                </svg>
              </a>
            </Button>
          ))}
        </div>
      </div>
    </div>
  )
}

/** A block the operator can copy in one click — the fallback is the text
    itself, which is selectable either way. */
function Copyable({ label, text }: { label: string; text: string }) {
  const { state, copy } = useCopyFlash(text)
  return (
    <div className="learn-prompt">
      <div className="learn-prompt-head">
        <span className="learn-prompt-label">{label}</span>
        <Button
          variant="ghost"
          size="sm"
          className="learn-ghost"
          onClick={copy}
          aria-label={state === 'copied' ? 'Copied' : 'Copy'}
        >
          {state === 'copied' ? (
            <Check className={uiClasses.icon} aria-hidden="true" />
          ) : (
            <Copy className={uiClasses.icon} aria-hidden="true" />
          )}
          {state === 'copied' ? 'Copied' : 'Copy'}
        </Button>
      </div>
      <p className="learn-prompt-text">{text}</p>
    </div>
  )
}

export function when(at: number): string {
  return new Date(at).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
}

function format(payload: unknown): string {
  try {
    return JSON.stringify(payload, null, 2) ?? 'null'
  } catch {
    return String(payload)
  }
}
