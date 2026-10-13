import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { createRoot } from 'react-dom/client'
import { CodeEditor } from '@/components/ui/CodeEditor'
import { FileDiff } from '@/components/ui/FileDiff'
import { TooltipProvider } from '@/components/ui/Tooltip'
import { useKeybindings } from '@/hooks/use-keybindings'
import { getPageCommands, registerPageCommands } from '@/lib/page-commands'
import { Workers } from '@/pages/Workers'
import { harness } from './mock-iii.js'
import '@/index.css'
import './preview.css'

const parameters = new URLSearchParams(location.search)
const paneId = 'workers-regression'
const commands = {
  register(commands) {
    return registerPageCommands({
      pageId: 'workers',
      pageTitle: 'Workers',
      source: 'page',
      paneId,
      commands,
    })
  },
}
harness.commands = () =>
  getPageCommands().map(({ command, bindings }) => ({
    id: command.id,
    bindings,
    enabled: command.enabled?.() ?? true,
  }))
harness.runCommand = (id) => {
  const command = getPageCommands().find(
    (entry) => entry.command.id === id,
  )?.command
  if (!command || command.enabled?.() === false)
    throw new Error(`Unavailable fixture command: ${id}`)
  command.run()
}
harness.editorSnapshot = async () => {
  const { monaco } = await import('@/lib/monaco')
  const editor = monaco.editor
    .getEditors()
    .find((editor) => editor.getDomNode()?.closest('.wk-code'))
  if (!editor) throw new Error('The real settings editor has not mounted')
  return {
    value: editor.getValue(),
    position: editor.getPosition(),
    selection: editor.getSelection(),
    scrollTop: editor.getScrollTop(),
    focused: editor.hasTextFocus(),
  }
}
document.documentElement.dataset.theme = parameters.get('theme') ?? 'dark'
const pane = parameters.get('pane')
if (pane)
  document.documentElement.style.setProperty(
    '--harness-pane-width',
    `${Number(pane)}px`,
  )

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false } },
})
const root = document.getElementById('root')
if (!root) throw new Error('missing regression harness root')

function Preview() {
  useKeybindings({})
  if (parameters.get('monacoCase') === 'fill') {
    return (
      <section
        aria-label="Native bounded editor regression"
        style={{ height: 360 }}
      >
        <CodeEditor
          className="wk-code"
          fill
          language="yaml"
          lineNumbers
          value={Array.from(
            { length: 100 },
            (_, index) => `setting_${index + 1}: ${index + 1}`,
          ).join('\n')}
          onChange={() => {}}
          readOnly
          aria-label="Bounded configuration YAML"
        />
      </section>
    )
  }
  if (parameters.get('monacoCase') === 'json') {
    return (
      <section aria-label="Native JSON punctuation regression">
        <CodeEditor
          className="wk-code"
          language="json"
          value={'{"port":3113,"enabled":true,"values":[1,2]}'}
          onChange={() => {}}
          readOnly
          aria-label="Registered configuration JSON"
        />
      </section>
    )
  }
  if (parameters.has('diffCase')) {
    const lines = Array.from(
      { length: 300 },
      (_, index) => `setting_${index + 1}: ${index + 1}`,
    )
    const oldFile = { name: 'worker-compose.yaml', contents: lines.join('\n') }
    lines[19] = 'setting_20: 200'
    lines[279] = 'setting_280: 2800'
    return (
      <section aria-label="Native diff regression" tabIndex={-1}>
        <FileDiff
          oldFile={oldFile}
          newFile={{ ...oldFile, contents: lines.join('\n') }}
          disableFileHeader
        />
      </section>
    )
  }
  return (
    <QueryClientProvider client={queryClient}>
      <TooltipProvider>
        <div className="workers-harness-pane" data-workspace-pane-id={paneId}>
          <Workers
            paneId={paneId}
            commands={parameters.has('commands') ? commands : undefined}
            setDirty={(dirty) => {
              harness.dirty = dirty
            }}
          />
        </div>
      </TooltipProvider>
    </QueryClientProvider>
  )
}

createRoot(root).render(<Preview />)
