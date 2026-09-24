import * as React from 'react'
import { Button } from '@/components/ui/Button'
import { Checkbox } from '@/components/ui/Checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from '@/components/ui/Dialog'
import { Input } from '@/components/ui/Input'
import { useEmailPrompt } from '@/hooks/use-email-prompt'
import { isEmailish } from '@/lib/email-prompt'
import { getIiiClient } from '@/lib/iii-client'

/** The onboarding worker owns the list; the console only hands it an address. */
const SUBSCRIBE_FN = 'onboarding::subscribe'
const SUBSCRIBE_TIMEOUT_MS = 20_000

/**
 * Asks once for an email address, after thirty minutes of console use.
 *
 * The browser sends the address to `onboarding::subscribe` over the engine
 * WebSocket and nowhere else. That worker adds it to the product update list
 * and announces it for the engine to write to this machine's person; the
 * browser never talks to the list or to the analytics vendor.
 */
export function EmailPrompt() {
  const { open, onSnooze, onDismiss, onSubscribed } = useEmailPrompt()
  const [email, setEmail] = React.useState('')
  const [never, setNever] = React.useState(false)
  const [sending, setSending] = React.useState(false)
  const [error, setError] = React.useState<string | null>(null)

  const close = () => {
    if (sending) return
    if (never) onDismiss()
    else onSnooze()
  }

  const submit = () => {
    if (sending || !isEmailish(email)) return
    setSending(true)
    setError(null)
    void (async () => {
      try {
        const client = await getIiiClient()
        await client.trigger(
          SUBSCRIBE_FN,
          { email: email.trim(), source: 'console_prompt' },
          { timeoutMs: SUBSCRIBE_TIMEOUT_MS },
        )
        onSubscribed()
      } catch (cause) {
        // A failed signup keeps the box open with the address intact, and the
        // record stays `pending` so nothing is silently lost.
        setError(
          cause instanceof Error ? cause.message : 'could not add that address',
        )
      } finally {
        setSending(false)
      }
    })()
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) close()
      }}
    >
      <DialogContent className="max-w-md">
        <DialogTitle className="pr-8 text-[14px]">Product updates</DialogTitle>
        <DialogDescription className="mt-2 text-[13px] leading-relaxed">
          iii moves quickly. If you&apos;d like to stay up to date on our work
          please enter your email for roadmap, product, and educational
          updates. We won&apos;t share your email with anyone else.
        </DialogDescription>
        <form
          className="mt-4 flex flex-col gap-3"
          onSubmit={(event) => {
            event.preventDefault()
            submit()
          }}
        >
          <Input
            type="email"
            autoComplete="email"
            placeholder="you@example.com"
            aria-label="Email address"
            value={email}
            onChange={(next) => {
              setEmail(next)
              setError(null)
            }}
          />
          {error ? (
            <p className="text-[12px] text-alert" role="alert">
              {error}
            </p>
          ) : null}
          <Checkbox
            label="Do not show this again"
            checked={never}
            onChange={(event) => setNever(event.currentTarget.checked)}
          />
          <div className="mt-1 flex items-center justify-end gap-2">
            <Button type="button" variant="ghost" onClick={close}>
              Not now
            </Button>
            <Button type="submit" disabled={sending || !isEmailish(email)}>
              {sending ? 'Adding...' : 'Keep me posted'}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  )
}
