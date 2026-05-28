interface FullPermissionsBannerProps {
  onDisable: () => void
}

/**
 * Persistent banner shown while a conversation is in `mode === 'full'`.
 * Always visible; the only way to dismiss is by reverting to manual.
 * Tone is intentionally loud — Full mode bypasses every safety prompt,
 * including destructive verbs like `shell::exec` and `fs::write`.
 */
export function FullPermissionsBanner({
  onDisable,
}: FullPermissionsBannerProps) {
  return (
    <div
      role="status"
      aria-live="polite"
      className="flex items-center justify-between gap-3 border-y border-alert bg-alert/10 px-4 py-2"
    >
      <p className="font-mono text-[12px] text-ink">
        <span className="font-semibold text-alert">
          full permissions active
        </span>
        <span className="ml-2 text-ink-faint">
          the agent runs every function without asking — including writing
          files, executing shells, and sending messages.
        </span>
      </p>
      <button
        type="button"
        onClick={onDisable}
        className="font-mono text-[12px] px-3 py-1 border border-ink bg-ink text-bg hover:bg-bg hover:text-ink transition-colors shrink-0"
      >
        disable
      </button>
    </div>
  )
}
