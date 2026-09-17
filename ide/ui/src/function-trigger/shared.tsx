/* What every shell::* card shares: the terminal-shaped card composed from
   the console's atoms (TerminalCommandLine, MetaRow, TerminalStream,
   EmptyState), the MetaRow item helpers, and the header label. */

import {
  EmptyState,
  MetaRow,
  type MetaRowItem,
  TerminalCommandLine,
  TerminalStream,
  uiClasses,
} from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { truncateMiddle } from '../lib/format'
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

/** One `label value` pair for a card's MetaRow. */
export const kv = (label: string, value: ReactNode): MetaRowItem => ({ label, value })

/** The truthy items, in order — views build their strip conditionally. */
export const items = (...list: Array<MetaRowItem | null | false | undefined>): MetaRowItem[] =>
  list.filter((item): item is MetaRowItem => !!item)

/** Where the call runs. Label `on` (avoids colliding with sed's `target`
    item); neutral for both kinds — most calls are host, and a permanently
    warn header reads as a perpetual alarm. `target` omitted on the wire
    means host (serde default). exec/exec_bg pass `explicitHost`: host is
    the privileged surface approval decisions hinge on, so the default gets
    an item there; fs views leave host implicit. */
export function targetItem(target: ShellTarget | undefined, explicitHost = false): MetaRowItem | null {
  if (isSandboxTarget(target)) return kv('on', `sandbox ${truncateMiddle(target.sandbox_id, 12)}`)
  return explicitHost ? kv('on', 'host') : null
}

/** Sandbox-target responses fill `path`/`src`/`dst` with `""` — prefer
    the canonicalised response path when present, else the request path. */
export function displayPath(reqPath: string, respPath?: string): string {
  return respPath && respPath.length > 0 ? respPath : reqPath
}

interface TerminalCardProps {
  /** The `$ command` line; omitted for calls that have no command. */
  command?: string
  /** Pulsing header + `triggering…` in place of the body. */
  running?: boolean
  /** Label/value pairs of the metadata strip. */
  items?: MetaRowItem[]
  /** Free-form chips/badges that trail the items. */
  chips?: ReactNode
  children?: ReactNode
  /** Footer pill row (exit code, duration, timed-out). */
  footer?: ReactNode
}

/** The terminal-shaped card: command line, metadata strip, body, footer. */
export function TerminalCard({ command, running, items, chips, children, footer }: TerminalCardProps) {
  return (
    <div className="shui-card">
      {command !== undefined ? (
        <div className={running ? `shui-card-head ${uiClasses.pulse}` : 'shui-card-head'}>
          <TerminalCommandLine command={command} copy />
        </div>
      ) : null}
      {items?.length || chips ? <MetaRow items={items}>{chips}</MetaRow> : null}
      {running ? <div className="shui-running">triggering…</div> : children}
      {footer ? <div className="shui-card-foot">{footer}</div> : null}
    </div>
  )
}

/** stdout above stderr — exec output is buffered, so the streams never
    interleave — and the shared empty state when both are blank. */
export function StreamBody({ stdout = '', stderr = '' }: { stdout?: string; stderr?: string }) {
  return (
    <div className="shui-card-body">
      {!stdout && !stderr ? (
        <EmptyState title="No output" description="Nothing was written to stdout or stderr." />
      ) : (
        <>
          <TerminalStream label="stdout" text={stdout} ansi />
          <TerminalStream label="stderr" text={stderr} tone="err" ansi />
        </>
      )}
    </div>
  )
}

/** The function id in the console's card header — faint namespace, ink
    tail. Renders outside the scope wrapper, hence inline token styles. */
export function FunctionIdLabel({ functionId }: { functionId: string }) {
  const split = functionId.indexOf('::')
  if (split < 0) return <span style={{ color: 'var(--color-ink)' }}>{functionId}</span>
  return (
    <>
      <span style={{ color: 'var(--color-ink-faint)' }}>{functionId.slice(0, split + 2)}</span>
      <span style={{ color: 'var(--color-ink)', fontWeight: 500 }}>{functionId.slice(split + 2)}</span>
    </>
  )
}
