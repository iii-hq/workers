import type { ReactNode } from 'react'
import { runCommand } from './save'

/** One effect sentence. The run line's command is one code node, never split on backticks: the person approves
 * exactly what runs. Any other sentence goes through `rest`. */
export function effectLine(line: string, rest: (line: string) => ReactNode = (l) => l): ReactNode {
  const command = runCommand(line)
  if (command === null) return rest(line)
  return (
    <>
      {line.slice(0, line.indexOf('`'))}
      <code className="mcp-ui-inline-code">{command}</code>
    </>
  )
}
