/**
 * What the signup box is allowed to send on.
 *
 * One address, an `@`, and a dot after it. The list itself does the real
 * validation; this only stops an obvious typo — or a pasted paragraph —
 * from becoming a POST to someone else's service.
 */

/** RFC 5321's ceiling for a whole address. */
export const MAX_EMAIL_LENGTH = 254

export const isEmailish = (value) => {
  if (typeof value !== 'string') return false
  const email = value.trim()
  if (email.length === 0 || email.length > MAX_EMAIL_LENGTH) return false
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)
}
