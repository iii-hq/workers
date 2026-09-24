import {
  type ComposerControlProps,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  type ExtensionIii,
} from '@iii-dev/console-ui'
import { useCallback, useState } from 'react'
import { listProviders, type RegisteredProvider } from '../configuration'

/**
 * Session metadata key the harness reads at the start of every turn and
 * stamps on the turn's context (`iii.judge.provider` baggage), so every judge
 * call that turn causes routes to it. Absent = the judge settings' default.
 */
export const SESSION_PROVIDER_KEY = 'judge_provider'
/** Radio value for "no session override". */
const DEFAULT_CHOICE = ''
type Engine = Pick<ExtensionIii, 'trigger'>

/** Bind the composer control to the console's engine client once. */
export function createJudgeSessionControl(iii: Engine) {
  return function JudgeSessionControl(props: ComposerControlProps) {
    return <JudgeSessionPicker {...props} iii={iii} />
  }
}

/**
 * The judge provider for THIS session, beside the model picker. Like the
 * model, a change applies from the session's next turn and never touches
 * other sessions or the judge's default.
 */
export function JudgeSessionPicker({ iii, metadata, setMetadata }: ComposerControlProps & { iii: Engine }) {
  const raw = metadata[SESSION_PROVIDER_KEY]
  const stored = typeof raw === 'string' && raw ? raw : undefined
  // null = not listed yet; the list loads when the menu opens.
  const [providers, setProviders] = useState<RegisteredProvider[] | null>(null)
  const [listError, setListError] = useState<string | null>(null)
  const load = useCallback(() => {
    setListError(null)
    listProviders(iii)
      .then(setProviders)
      .catch((error: unknown) => {
        setListError(error instanceof Error ? error.message : String(error))
        setProviders((current) => current ?? [])
      })
  }, [iii])
  const names = (providers ?? []).map((entry) => entry.provider)
  if (stored && providers && !names.includes(stored)) names.push(stored)
  const choose = (value: string) => setMetadata({ [SESSION_PROVIDER_KEY]: value === DEFAULT_CHOICE ? undefined : value })

  return (
    <DropdownMenu
      onOpenChange={(open) => {
        if (open) load()
      }}
    >
      <DropdownMenuTrigger
        className="judge-ui-session-trigger"
        aria-label={`Judge provider for this session: ${stored ?? 'default'}`}
        title="Judge provider for this session"
      >
        judge · {stored ?? 'default'}
      </DropdownMenuTrigger>
      <DropdownMenuContent side="top" align="end">
        <DropdownMenuLabel>Judge for this session</DropdownMenuLabel>
        <DropdownMenuRadioGroup value={stored ?? DEFAULT_CHOICE} onValueChange={choose}>
          <DropdownMenuRadioItem value={DEFAULT_CHOICE}>Default (judge settings)</DropdownMenuRadioItem>
          {providers === null ? (
            <DropdownMenuItem disabled>Checking providers…</DropdownMenuItem>
          ) : (
            names.map((name) => (
              <DropdownMenuRadioItem key={name} value={name}>
                {providers.some((entry) => entry.provider === name) ? name : `${name} (not running)`}
              </DropdownMenuRadioItem>
            ))
          )}
        </DropdownMenuRadioGroup>
        {listError ? (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem disabled>Could not list providers: {listError}</DropdownMenuItem>
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
