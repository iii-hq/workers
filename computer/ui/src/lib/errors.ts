/** Any thrown value → a message worth showing in the page. */
export function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message
  if (typeof err === 'string') return err
  try {
    // `JSON.stringify` answers undefined for a function or a bare undefined.
    return JSON.stringify(err) ?? String(err)
  } catch {
    return String(err)
  }
}
