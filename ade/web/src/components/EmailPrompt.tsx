import { Loader2, Mail, MailCheck } from 'lucide-react'
import * as React from 'react'
import { Input } from '@/components/shadcn/input'
import { Button } from '@/components/ui/Button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { useEmailPrompt } from '@/hooks/use-email-prompt'
import { isEmailish } from '@/lib/email-prompt'
import { getIiiClient } from '@/lib/iii-client'

/** This worker owns the signup; the browser only hands it an address. */
const SUBSCRIBE_FN = 'ade::subscribe'
const SUBSCRIBE_TIMEOUT_MS = 20_000

const INVALID_EMAIL = 'Enter an email address like name@example.com'

/** `ade::subscribe`'s answer. `outcome` is absent from workers older than it. */
interface SubscribeResult {
  subscribed?: boolean
  outcome?: 'subscribed' | 'invalid_email' | 'retry_later'
}

/**
 * Asks once for an email address, after thirty minutes of console use.
 *
 * Two steps in one dialog: the address, then a "check your inbox" note,
 * because the list only sends updates once the address is confirmed from the
 * email it sends. Laid out like `ConfirmDialog` (Inter, 16px title, 14px body,
 * actions on their own band) so it reads as part of the same console.
 *
 * The browser sends the address to `ade::subscribe` over the engine
 * WebSocket and nowhere else. The ADE worker adds it to the product update
 * list and announces it for the engine to write to this machine's person; the
 * browser never sends the address to the list or to the analytics vendor.
 *
 * Only an address the list refuses is shown as an error. Anything else that
 * stops the signup (the list not open to the ADE yet, rate limits, outages, a
 * worker that is not connected) closes the prompt quietly and leaves it to ask
 * again on a later run.
 */
export function EmailPrompt() {
  const { open, onSnooze, onDismiss, onSubscribed, onRetryLater, onClose } =
    useEmailPrompt()
  const [email, setEmail] = React.useState('')
  // The address the list took. Set, the dialog asks for the confirmation
  // click instead of an address.
  const [sentTo, setSentTo] = React.useState<string | null>(null)
  const [sending, setSending] = React.useState(false)
  const [error, setError] = React.useState<string | null>(null)
  const inputRef = React.useRef<HTMLInputElement>(null)
  const errorId = React.useId()

  /** Closed without a decision (Not now, ×, Escape, a click outside). */
  const close = () => {
    if (sending) return
    if (sentTo) onClose()
    else onSnooze()
  }

  const submit = () => {
    if (sending || !isEmailish(email)) return
    setSending(true)
    setError(null)
    void (async () => {
      let result: SubscribeResult | null = null
      try {
        const client = await getIiiClient()
        result = await client.call<SubscribeResult>(
          SUBSCRIBE_FN,
          { email: email.trim(), source: 'console_prompt' },
          { timeoutMs: SUBSCRIBE_TIMEOUT_MS },
        )
      } catch (cause) {
        // The worker is missing or did not answer in time. Nothing the person
        // can fix, so it waits for a later run like any other outage.
        console.warn('[email-prompt] signup did not complete', cause)
      } finally {
        setSending(false)
      }
      // The list is called from the ADE worker, not this page, so the Network
      // tab never shows it. This line is the browser's record of the answer.
      console.info(
        `[email-prompt] ${SUBSCRIBE_FN} answered: ${result?.outcome ?? 'no answer'}`,
      )
      if (result?.outcome === 'invalid_email') {
        // The box stays open with the address intact so it can be corrected;
        // the effect below puts the cursor back once the field is enabled.
        setError(INVALID_EMAIL)
      } else if (result?.subscribed) {
        // The list only sends updates once the address is confirmed from the
        // email it just sent, so say so before closing.
        onSubscribed()
        setSentTo(email.trim())
      } else {
        onRetryLater()
      }
    })()
  }

  // A refused address sends the cursor back to it. This runs after the render
  // that re-enables the field: focus() on a disabled input does nothing.
  React.useEffect(() => {
    if (error && !sending) inputRef.current?.focus()
  }, [error, sending])

  /** Back to the address, kept as typed, to fix a typo the note revealed. */
  const changeAddress = () => {
    setSentTo(null)
    requestAnimationFrame(() => inputRef.current?.select())
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close()
      }}
    >
      <DialogContent
        className="flex w-[calc(100%-24px)] max-w-[420px] flex-col overflow-hidden rounded-2xl p-0 font-inter"
        onOpenAutoFocus={(event) => {
          // The address field, not the close button, takes the first focus.
          event.preventDefault()
          inputRef.current?.focus()
        }}
      >
        {sentTo ? (
          <div
            key="sent"
            className="flex flex-col motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-bottom-1 motion-safe:duration-200"
          >
            <div className="px-6 pt-6 pb-5">
              <span
                aria-hidden="true"
                className="flex size-10 items-center justify-center rounded-xl bg-ok-muted text-ok"
              >
                <MailCheck className="size-5" strokeWidth={1.75} />
              </span>
              <DialogTitle className="mt-4 pr-8 font-inter text-[16px] leading-6 font-semibold">
                Check your inbox
              </DialogTitle>
              <DialogDescription className="mt-1.5 font-inter text-[14px] leading-[22px] text-pretty text-ink-faint">
                We sent a confirmation link to{' '}
                <span className="font-medium break-all text-ink">{sentTo}</span>
                . Click it to confirm, and the updates will start.
              </DialogDescription>
              <p className="mt-3 font-inter text-[13px] leading-5 text-ink-faint">
                Can&apos;t find it? Check your spam folder. Already confirmed
                this address? You&apos;re all set.
              </p>
            </div>
            <div className="flex shrink-0 flex-wrap items-center justify-between gap-2 bg-surface px-6 py-4">
              <Button
                type="button"
                variant="ghost"
                className="-ml-2 h-9 px-2 text-[14px]"
                onClick={changeAddress}
              >
                Use a different email
              </Button>
              <Button
                type="button"
                variant="primary"
                className="h-9 text-[14px]"
                onClick={onClose}
                autoFocus
              >
                Done
              </Button>
            </div>
          </div>
        ) : (
          <form
            key="ask"
            noValidate
            className="flex flex-col motion-safe:animate-in motion-safe:fade-in-0 motion-safe:duration-200"
            onSubmit={(event) => {
              event.preventDefault()
              submit()
            }}
          >
            <div className="px-6 pt-6 pb-5">
              <span
                aria-hidden="true"
                className="flex size-10 items-center justify-center rounded-xl bg-surface text-ink ring-1 ring-dialog-edge"
              >
                <Mail className="size-5" strokeWidth={1.75} />
              </span>
              <DialogTitle className="mt-4 pr-8 font-inter text-[16px] leading-6 font-semibold">
                Get iii product updates
              </DialogTitle>
              <DialogDescription className="mt-1.5 font-inter text-[14px] leading-[22px] text-pretty text-ink-faint">
                Roadmap news, releases and guides from the iii team. We
                won&apos;t share your email with anyone.
              </DialogDescription>
              <Input
                ref={inputRef}
                type="email"
                name="email"
                autoComplete="email"
                inputMode="email"
                spellCheck={false}
                placeholder="name@example.com"
                aria-label="Email address"
                aria-invalid={error ? true : undefined}
                aria-describedby={error ? errorId : undefined}
                disabled={sending}
                value={email}
                onChange={(event) => {
                  setEmail(event.currentTarget.value)
                  setError(null)
                }}
                className="mt-4 h-10 rounded-lg px-3 font-inter text-ink placeholder:text-ink-faint"
              />
              {error ? (
                <p
                  id={errorId}
                  role="alert"
                  className="mt-2 font-inter text-[13px] leading-5 text-alert"
                >
                  {error}
                </p>
              ) : null}
            </div>
            <div className="flex shrink-0 flex-wrap items-center justify-between gap-2 bg-surface px-6 py-4">
              <Button
                type="button"
                variant="ghost"
                className="-ml-2 h-9 px-2 text-[14px]"
                disabled={sending}
                onClick={onDismiss}
              >
                Don&apos;t ask again
              </Button>
              <div className="flex items-center gap-2">
                <Button
                  type="button"
                  variant="secondary"
                  className="h-9 text-[14px]"
                  disabled={sending}
                  onClick={close}
                >
                  Not now
                </Button>
                <Button
                  type="submit"
                  variant="primary"
                  className="h-9 min-w-[104px] text-[14px]"
                  disabled={sending || !isEmailish(email)}
                  aria-busy={sending || undefined}
                >
                  {sending ? (
                    <>
                      <Loader2
                        aria-hidden="true"
                        className="size-4 motion-safe:animate-spin"
                      />
                      Subscribing
                    </>
                  ) : (
                    'Subscribe'
                  )}
                </Button>
              </div>
            </div>
          </form>
        )}
      </DialogContent>
    </Dialog>
  )
}
