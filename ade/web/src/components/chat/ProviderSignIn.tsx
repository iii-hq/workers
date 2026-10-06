import { SquareTerminal } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Chip } from '@/components/ui/Chip'
import { scanMachine } from '@/lib/onboarding/api'
import { SUBSCRIPTION_PROVIDERS } from '@/lib/onboarding/catalog'
import type { ToolScan } from '@/lib/onboarding/plan'

/** One scan per page: the CLIs and their sign-in do not change under a form. */
let scan: Promise<ToolScan[]> | null = null

function scanOnce(): Promise<ToolScan[]> {
  scan ??= scanMachine().catch(() => {
    scan = null
    return []
  })
  return scan
}

/**
 * Authentication for a provider that signs in on its own (no API key): a
 * subscription provider shows the local CLI sign-in it uses, the same fact
 * the setup wizard shows; any other one says where its login lives.
 */
export function ProviderSignIn({ providerId }: { providerId: string }) {
  const subscription = SUBSCRIPTION_PROVIDERS.find(
    (entry) => entry.providerId === providerId,
  )
  const [tool, setTool] = useState<ToolScan | null | undefined>(
    subscription ? undefined : null,
  )

  useEffect(() => {
    if (!subscription) return
    let cancelled = false
    void scanOnce().then((tools) => {
      if (!cancelled) {
        setTool(tools.find((entry) => entry.id === subscription.toolId) ?? null)
      }
    })
    return () => {
      cancelled = true
    }
  }, [subscription])

  return (
    <section
      aria-label="Sign-in"
      className="flex flex-col gap-1.5 rounded-md bg-surface p-3"
    >
      <header className="flex items-center justify-between gap-2">
        <span className="flex min-w-0 items-center gap-2">
          <SquareTerminal
            className="size-4 shrink-0 text-ink-faint"
            aria-hidden
          />
          <span className="font-sans text-[13px] font-medium text-ink">
            Sign-in
          </span>
        </span>
        {subscription && tool !== undefined ? (
          <Chip tone={tool?.signed_in ? 'success' : 'warning'}>
            {tool?.signed_in ? 'Signed in' : 'Not signed in'}
          </Chip>
        ) : null}
      </header>
      <p className="font-sans text-[12px] leading-relaxed text-ink-faint">
        {subscription
          ? tool?.signed_in
            ? `Uses your ${subscription.title} sign-in on this machine — ${subscription.plan}, no API key.`
            : tool?.installed
              ? `${subscription.title} is installed but not signed in. Sign in with its CLI on this machine, then refresh the models.`
              : `Uses a ${subscription.title} sign-in on this machine; the CLI was not found here.`
          : 'This provider signs in on its own; there is no API key to set here.'}
      </p>
      {tool?.credentials_path ? (
        <span className="font-mono text-[11px] text-ink-ghost">
          {tool.credentials_path}
        </span>
      ) : null}
    </section>
  )
}
