export const RPC_TIMEOUT_MS = 10_000

/**
 * Bound Console RPC calls so a reconnect cannot keep a refresh gate occupied.
 * The underlying host transport owns cancellation and reconnection.
 *
 * @template T
 * @param {PromiseLike<T> | T} task
 * @param {string} label
 * @param {number} [timeoutMs]
 * @returns {Promise<T>}
 */
export async function withRpcTimeout(task, label, timeoutMs = RPC_TIMEOUT_MS) {
  /** @type {ReturnType<typeof setTimeout> | undefined} */
  let timer
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(
      () => reject(new Error(`${label} timed out after ${timeoutMs / 1000}s`)),
      timeoutMs,
    )
  })

  try {
    return /** @type {Promise<T>} */ (
      await Promise.race([Promise.resolve(task), timeout])
    )
  } finally {
    if (timer !== undefined) clearTimeout(timer)
  }
}
