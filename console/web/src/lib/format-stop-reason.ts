/**
 * Render a `stop-reason` StreamEvent as user-readable notice text.
 * Used when an assistant turn ends abnormally (max_tokens hit, provider
 * error, abort) so the user sees a clear cause instead of a silently
 * truncated reply.
 */
export type StopReason = 'length' | 'error' | 'aborted' | 'function_call'

export function formatStopReason(reason: StopReason, message?: string): string {
  switch (reason) {
    case 'length':
      return (
        'response truncated — the model hit its max output tokens before finishing. ' +
        "increase the provider worker's `default_max_tokens` (or the model's `max_output_tokens` in the catalog), " +
        'or send "continue" to resume.'
      )
    case 'error':
      return message
        ? `response failed: ${message}`
        : 'response failed mid-stream (no error message from the provider).'
    case 'aborted':
      return 'response aborted before completion.'
    case 'function_call':
      return 'response paused for tool call.'
  }
}
