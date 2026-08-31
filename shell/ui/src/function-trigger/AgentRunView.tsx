/**
 * Agents run in terminals, so an agent's chat card opens one.
 *
 * Shell owns the terminal surface, so shell — not each agent worker, and not
 * the console — puts this control on an agent run card. The renderer matches
 * ANOTHER worker's function ids on purpose: the alternative is the same button
 * copied into pi, claude-code, codex, opencode, grok, acp and devin.
 *
 * It opens a PTY rooted at the directory the run worked in. What the user types
 * there is the agent's own CLI, unwrapped and fully interactive — the console
 * adds a terminal, not a reimplementation of one.
 */

import { Button, type FunctionTriggerMessage, type FunctionTriggerRenderer, type Host } from '@iii-dev/console-ui'
import { SquareTerminal } from 'lucide-react'

/** Agent function id → the CLI that agent is, for the hint on the control. */
const AGENT_CLI: Record<string, string> = {
  pi: 'pi',
  // The claude-code worker registers under `claude::*`, not its worker name.
  claude: 'claude',
  codex: 'codex',
  opencode: 'opencode',
  grok: 'grok',
  devin: 'devin',
}

const RUN_SUFFIXES = ['::run', '::start', '::follow_up']

export function isAgentRunFunction(functionId: string): boolean {
  const marker = functionId.indexOf('::')
  if (marker <= 0) return false
  if (!(functionId.slice(0, marker) in AGENT_CLI)) return false
  return RUN_SUFFIXES.some((suffix) => functionId.endsWith(suffix))
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as Record<string, unknown>) : null
}

function cwdOf(message: FunctionTriggerMessage): string {
  const cwd = asRecord(message.input)?.cwd
  return typeof cwd === 'string' ? cwd : ''
}

function cliOf(functionId: string): string {
  const marker = functionId.indexOf('::')
  return marker > 0 ? (AGENT_CLI[functionId.slice(0, marker)] ?? '') : ''
}

/** The console has no error field on the message: a failed agent run says so
 *  in its own result shape (`is_error`, or an `error` the worker returned). */
function failed(message: FunctionTriggerMessage): boolean {
  const output = asRecord(message.output)
  if (!output) return false
  return output.is_error === true || typeof output.error === 'string'
}

function OpenAgentTerminal({ host, cwd, command }: { host: Host; cwd: string; command: string }) {
  return (
    <div className="shell-agent-card-actions">
      <Button
        variant="ghost"
        size="sm"
        title={
          command
            ? `Open a terminal in ${cwd} — run \`${command}\` there to drive it yourself`
            : `Open a terminal in ${cwd}`
        }
        onClick={() =>
          host.panels?.open({
            pageId: 'shell',
            context: { type: 'agent-terminal', cwd, command },
          })
        }
      >
        <SquareTerminal size={16} />
        Open terminal here
      </Button>
    </div>
  )
}

export function createAgentRunRenderer(host: Host): FunctionTriggerRenderer {
  const control = (message: FunctionTriggerMessage): React.ReactNode | null => {
    // Reject before creating a React element. Returning an element whose
    // component later renders null would still win the host's first-non-null
    // dispatch and hide the renderer that actually owns this function.
    if (!isAgentRunFunction(message.functionId)) return null
    // A failed call belongs to the host card: its error text is the thing worth
    // reading, and replacing it with a button hides why the run died.
    if (failed(message)) return null
    const cwd = cwdOf(message)
    // Without a directory there is nothing to root a terminal at, and an older
    // console has no contextual panels to open the shell page into.
    if (!cwd || !host.panels) return null
    return <OpenAgentTerminal host={host} cwd={cwd} command={cliOf(message.functionId)} />
  }
  return {
    id: 'shell/page.js#agent-run',
    isMatch: isAgentRunFunction,
    tryRenderRunning: control,
    tryRender: control,
    metadata: { display: true },
  }
}
