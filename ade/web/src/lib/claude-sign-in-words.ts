/**
 * The words of a Claude Code account's sign-in in the ADE (claude-login-ui
 * §5, as the apps' `SignInWords`): open the page, paste its code. A browser
 * is clicked, not tapped, so the lines that name the gesture say "click".
 */

/** The worker's link lasts 10 minutes. */
export const DEFAULT_LINK_MS = 10 * 60 * 1000

export const TITLE = 'Sign in to Claude Code'
export const STEP_OPEN = 'Open the sign-in page'
export const OPEN = 'Open in browser'
export const COPY_LINK = 'Copy link'
export const LINK_COPIED = 'Link copied'
export const STEP_PASTE = 'Paste the code'
export const PLACEHOLDER = 'The code from the page'
export const SIGN_IN = 'Sign in'
export const SIGNING_IN = 'Signing in…'
export const GETTING_LINK = 'Getting a sign-in link…'
export const CANCEL = 'Cancel'
export const CLOSE = 'Close'
export const DONE = 'Done'
export const NOT_NOW = 'Not now'
export const NEW_LINK = 'Get a new link'
export const TRY_AGAIN = 'Try again'
export const EXPIRED_TITLE = 'The sign-in link expired'
export const CANCELLED_TITLE = 'This sign-in stopped'
export const FAILED_TITLE = "Claude Code couldn't sign in"
export const START_FAILED_TITLE = "Couldn't get a sign-in link"
export const NO_REASON = 'The worker gave no reason.'
/** An account's menu while a sign-in for it is under way. */
export const CANCEL_SIGN_IN = 'Cancel sign-in'
export const REJECTED =
  "Anthropic didn't accept this code. A code works once, with the link it came from: open the link again for a new one."

function plural(n: number, word: string): string {
  return n === 1 ? `1 ${word}` : `${n} ${word}s`
}

/** "Personal · new account", "Personal · sign in again". */
export function subtitle(name: string, isNew: boolean, again: boolean): string {
  const what = isNew ? 'new account' : again ? 'sign in again' : 'sign in'
  return `${name} · ${what}`
}

/** Step 1's line: the page opens with the account's email, or for the account to choose. */
export function openBody(name: string, email: string | null): string {
  return email
    ? `It opens in your browser with ${email} filled in. Then click Copy Code on the page.`
    : `It opens in your browser. Sign in with the Claude account ${name} should use, then click Copy Code on the page.`
}

/** "10 minutes"; a link under 90 s in seconds. */
export function duration(ms: number): string {
  const s = Math.max(1, Math.floor((ms + 500) / 1000))
  if (s < 90) return plural(s, 'second')
  return plural(Math.floor((s + 30) / 60), 'minute')
}

/** "The link works for 10 minutes." */
export function linkWorks(linkMs: number): string {
  return `The link works for ${duration(linkMs)}.`
}

/** "9:41": what is left of a link, m:ss, never more than the worker's 10 minutes. */
export function left(expiresAtMs: number, nowMs: number): string {
  const s = Math.min(
    DEFAULT_LINK_MS / 1000,
    Math.max(0, Math.floor((expiresAtMs - nowMs + 999) / 1000)),
  )
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

/** Checking's line. */
export const CHECKING =
  'Claude Code is checking the code with Anthropic on the engine. This takes a few seconds.'

/** Expired's line. */
export function expiredBody(linkMs: number, email: string | null): string {
  return `Nothing came back in ${duration(linkMs)}, so Claude Code stopped waiting. ${
    email
      ? `Get a new link; it opens for ${email}.`
      : 'Get a new link to sign in.'
  }`
}

/** Cancelled's line: the worker's sentence, else ours. */
export function cancelledBody(name: string, said?: string | null): string {
  return (
    said?.trim() ||
    `It was cancelled, or a new sign-in for ${name} started on another device. Get a new link to sign in here.`
  )
}

/** Done's title. */
export function signedIn(name: string): string {
  return `${name} is signed in`
}

/** "Signed in as bo@example.com instead of ana@example.com."; null when the same. */
export function otherPerson(
  asked: string | null | undefined,
  signed: string | null | undefined,
): string | null {
  if (!asked || !signed || asked.toLowerCase() === signed.toLowerCase()) {
    return null
  }
  return `Signed in as ${signed} instead of ${asked}.`
}

/** Done's offer when the account isn't the global one. */
export function offer(
  name: string,
  globalName: string,
  globalWeek: number | null,
): string {
  return `${globalName} is the global account${
    globalWeek === null ? '' : ` (${globalWeek}% this week)`
  }. Make ${name} global so every Claude Code that isn't pinned uses it from its next message.`
}

/** Done when it is the global account already. */
export function isGlobal(name: string): string {
  return `${name} is the global account: every Claude Code that isn't pinned uses it.`
}

/** "Make Work global". */
export function makeGlobal(name: string): string {
  return `Make ${name} global`
}
