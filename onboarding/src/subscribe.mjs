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

/**
 * iii's own signup API. It sends a confirmation email itself; nobody gets
 * updates until they click it.
 */
export const DEFAULT_SIGNUP_URL = 'https://iii.dev/api/subscribe'

/**
 * The `source` the API knows the console by. The tour runs inside the ADE,
 * and the API refuses a source it has not opened to, so it is fixed here.
 */
export const SIGNUP_SOURCE = 'ade'

/**
 * Whether a signup URL may carry the operator's address: https anywhere, or
 * plain http to this machine so a local stand-in for the API can be tested
 * against (nothing leaves the machine).
 */
export const isSignupUrlAllowed = (value) => {
  const url = URL.parse(String(value))
  if (!url) return false
  if (url.protocol === 'https:') return true
  return url.protocol === 'http:' && ['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname)
}

/** The `error` code out of a JSON body like `{ "error": "invalid_email" }`. */
export const errorCode = (body) => {
  try {
    const parsed = JSON.parse(body)
    return typeof parsed?.error === 'string' ? parsed.error : null
  } catch {
    return null
  }
}

/**
 * What the API's answer means for the page:
 * - `subscribed`: a confirmation email went out, or the address was already
 *   on the list (both are a 200).
 * - `invalid_email`: the API refused the address itself; the person can fix it.
 * - `retry_later`: anything else (not open to this source yet, rate limited,
 *   down). Nothing for the person to fix; the step asks again another time.
 */
export const signupOutcome = (status, error) => {
  if (status >= 200 && status < 300) return 'subscribed'
  if (status === 400 && error === 'invalid_email') return 'invalid_email'
  return 'retry_later'
}
