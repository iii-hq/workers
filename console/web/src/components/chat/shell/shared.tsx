import { truncateMiddle } from '@/components/chat/sandbox/format'
import { Chip } from '@/components/chat/sandbox/terminal/Terminal'
import type { ShellTarget } from './parsers'

/** Narrow a request `target` to the sandbox variant. Also the views'
    "suppress serde fill-ins" signal — sandbox-target responses blank
    `path`/`src`/`dst` and default `already_existed`/`was_present`/
    `overwrote`, so those pills are meaningless there. */
export function isSandboxTarget(
  target: ShellTarget | undefined,
): target is Extract<ShellTarget, { kind: 'sandbox' }> {
  return target?.kind === 'sandbox'
}

interface TargetChipProps {
  target?: ShellTarget
  /** exec/exec_bg (and their previews) set this: host is the privileged
      execution surface approval decisions hinge on, so the default gets
      an explicit chip. fs views omit it — host stays implicit and only
      the sandbox deviation gets a chip. */
  explicitHost?: boolean
}

/** Where the call runs. Label `on` (avoids colliding with sed's `target`
    chip); neutral tone for both kinds — most calls are host, and a
    permanently warn header reads as a perpetual alarm. `target` omitted
    on the wire means host (serde default); normalised here so views
    don't repeat the rule. */
export function TargetChip({ target, explicitHost }: TargetChipProps) {
  if (isSandboxTarget(target)) {
    return (
      <Chip label="on">{`sandbox ${truncateMiddle(target.sandbox_id, 12)}`}</Chip>
    )
  }
  if (!explicitHost) return null
  return <Chip label="on">host</Chip>
}

/** Sandbox-target responses fill `path`/`src`/`dst` with `""` — prefer
    the canonicalised response path when present, else the request path. */
export function displayPath(reqPath: string, respPath?: string): string {
  return respPath && respPath.length > 0 ? respPath : reqPath
}
