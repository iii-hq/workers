import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { effectLine } from '../src/effect-line'
import { effectSentences, runCommand } from '../src/save'

describe('effectLine', () => {
  it('shows a command with backticks exactly as it runs, as one code node', () => {
    const runs = 'sh -c "echo `id`" `'
    const [line, send] = effectSentences({ runs, sends: [{ ref: 'secret://K', host: 'example.com' }] })
    expect(line).toBe(`mcp-client will run \`${runs}\``)
    expect(runCommand(line)).toBe(runs)
    expect(renderToStaticMarkup(effectLine(line))).toBe(
      'mcp-client will run <code class="mcp-ui-inline-code">sh -c &quot;echo `id`&quot; `</code>',
    )
    expect(runCommand(send)).toBe(null)
    expect(renderToStaticMarkup(effectLine(send))).toBe('mcp-client will send K to example.com')
    // The run line never reaches `rest`, which may split on backticks.
    expect(renderToStaticMarkup(effectLine(line, () => 'split'))).not.toContain('split')
  })
})
