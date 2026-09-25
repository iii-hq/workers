/**
 * When to ask for an email address, and whether to ask again.
 *
 * Console usage, not harness usage: the clock runs whenever this tab is on
 * screen, whether or not an agent is doing anything. Thirty minutes of that
 * earns one prompt.
 *
 * localStorage only. No engine state and no worker config, so the record is
 * per browser profile and a second machine asks again. Two tabs of the same
 * browser share the key, which is what keeps them from both asking.
 */

const KEY = 'iii-email-prompt'

/** Active time on screen before the prompt is earned. */
export const PROMPT_AFTER_MS = 30 * 60 * 1000

/** A prompt closed without a decision waits at least this long. */
export const SNOOZE_MS = 36 * 60 * 60 * 1000

export type EmailPromptStatus =
  /** Not asked yet, or asked and closed without a decision. */
  | 'pending'
  /** "Do not show again" was ticked. Never ask again. */
  | 'dismissed'
  /** An address was accepted. Never ask again. */
  | 'subscribed'

export interface EmailPromptRecord {
  status: EmailPromptStatus
  /** Active milliseconds accumulated on screen. */
  activeMs: number
  /** Epoch millis before which the prompt stays closed. */
  snoozeUntil: number
}

export const emptyRecord = (): EmailPromptRecord => ({
  status: 'pending',
  activeMs: 0,
  snoozeUntil: 0,
})

const isStatus = (value: unknown): value is EmailPromptStatus =>
  value === 'pending' || value === 'dismissed' || value === 'subscribed'

const finiteAt = (value: unknown, min: number): number =>
  typeof value === 'number' && Number.isFinite(value) && value > min
    ? value
    : min

/** Parse a stored record, falling back to a fresh one on anything unexpected. */
export function parseRecord(raw: string | null): EmailPromptRecord {
  if (!raw) return emptyRecord()
  try {
    const parsed: unknown = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object') return emptyRecord()
    const record = parsed as Partial<EmailPromptRecord>
    return {
      status: isStatus(record.status) ? record.status : 'pending',
      activeMs: finiteAt(record.activeMs, 0),
      snoozeUntil: finiteAt(record.snoozeUntil, 0),
    }
  } catch {
    return emptyRecord()
  }
}

export function loadRecord(): EmailPromptRecord {
  try {
    return parseRecord(localStorage.getItem(KEY))
  } catch {
    // Private windows and blocked site data both throw here. Asking nobody is
    // better than asking on every load.
    return { ...emptyRecord(), status: 'dismissed' }
  }
}

export function saveRecord(record: EmailPromptRecord): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(record))
  } catch {
    /* best-effort */
  }
}

/** Whether this record has earned a prompt right now. */
export function shouldPrompt(record: EmailPromptRecord, now: number): boolean {
  return (
    record.status === 'pending' &&
    record.activeMs >= PROMPT_AFTER_MS &&
    now >= record.snoozeUntil
  )
}

/**
 * Add time on screen.
 *
 * A tick longer than the interval it was scheduled for means the tab was
 * asleep, so it counts as one interval rather than the wall-clock gap. A
 * laptop shut overnight must not wake up owing thirty minutes.
 */
export function addActiveTime(
  record: EmailPromptRecord,
  elapsedMs: number,
  maxTickMs: number,
): EmailPromptRecord {
  if (record.status !== 'pending' || elapsedMs <= 0) return record
  return {
    ...record,
    activeMs: record.activeMs + Math.min(elapsedMs, maxTickMs),
  }
}

/** Closed without a decision: ask again after the snooze, not before. */
export function snooze(
  record: EmailPromptRecord,
  now: number,
): EmailPromptRecord {
  return { ...record, snoozeUntil: now + SNOOZE_MS }
}

export const dismiss = (record: EmailPromptRecord): EmailPromptRecord => ({
  ...record,
  status: 'dismissed',
})

export const subscribed = (record: EmailPromptRecord): EmailPromptRecord => ({
  ...record,
  status: 'subscribed',
})

/** RFC 5321's ceiling for a whole address. */
export const MAX_EMAIL_LENGTH = 254

/**
 * What the signup box is allowed to send on.
 *
 * One address: a local part that neither starts with a dot nor carries two in
 * a row and does not end on one, then a dotted domain ending in an alphabetic
 * TLD. The list and the engine both check again; this only stops an obvious
 * typo from becoming a request.
 */
const EMAIL =
  /^(?!\.)(?!.*\.\.)([a-z0-9_'+\-.]*)[a-z0-9_+-]@([a-z0-9][a-z0-9-]*\.)+[a-z]{2,}$/i

export function isEmailish(value: string): boolean {
  const email = value.trim()
  if (email.length === 0 || email.length > MAX_EMAIL_LENGTH) return false
  return EMAIL.test(email)
}
