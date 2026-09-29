import { Button } from '@/components/ui/Button'

export function ConversationLoadNotice({
  loading,
  error,
  onRetry,
}: {
  loading: boolean
  error?: string | null
  onRetry: () => void
}) {
  if (!loading && !error) return null
  return (
    <div
      role={error ? 'alert' : 'status'}
      className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-b border-edge bg-panel px-4 py-3 font-sans text-sm text-ink"
    >
      <div className="min-w-0 flex-1">
        <p className="font-medium">{error || 'Loading conversations…'}</p>
        {error ? (
          <p className="text-ink-faint">
            Existing messages are still shown where available. Try loading
            again.
          </p>
        ) : null}
      </div>
      {error ? (
        <Button
          type="button"
          variant="pill"
          className="min-h-11"
          onClick={onRetry}
        >
          Try again
        </Button>
      ) : null}
    </div>
  )
}
