import type { Meta, StoryObj } from '@storybook/react-vite'
import { MessageSquareQuote } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { fn } from 'storybook/test'
import { CodeEditor, type CodeEditorHandle } from './CodeEditor'

const LONG_FILE = Array.from(
  { length: 200 },
  (_, index) => `fn step_${index + 1}() {\n    // line ${index + 1}\n}\n`,
).join('\n')

function RevealOnMount({ line }: { line: number }) {
  const handle = useRef<CodeEditorHandle>(null)
  const [value, setValue] = useState(LONG_FILE)
  useEffect(() => {
    handle.current?.revealLine(line)
  }, [line])
  return (
    <div className="h-[480px] w-[720px] overflow-auto border border-edge">
      <CodeEditor
        ref={handle}
        value={value}
        onChange={setValue}
        language="rust"
        aria-label="reveal demo"
      />
    </div>
  )
}

/** Select lines 4–9 on mount: the way a `#file(path:4-9)` reference opens. */
function RevealLinesDemo({ from, to }: { from: number; to: number }) {
  const handle = useRef<CodeEditorHandle>(null)
  const [value, setValue] = useState(LONG_FILE)
  useEffect(() => {
    handle.current?.revealLines(from, to)
  }, [from, to])
  return (
    <div className="h-[420px] w-[720px] border border-edge">
      <CodeEditor
        ref={handle}
        value={value}
        onChange={setValue}
        language="rust"
        aria-label="reveal lines demo"
        fill
        lineNumbers
      />
    </div>
  )
}

/** Select some text: a discreet "Reference in chat" bar appears by the end
    of the selection (the action is logged to the Actions panel). */
function SelectionActions() {
  const [value, setValue] = useState(LONG_FILE)
  return (
    <div className="h-[420px] w-[720px] border border-edge">
      <CodeEditor
        value={value}
        onChange={setValue}
        language="rust"
        aria-label="selection actions demo"
        fill
        lineNumbers
        selectionActions={[
          {
            id: 'reference-in-chat',
            label: 'Reference in chat',
            icon: <MessageSquareQuote aria-hidden />,
            run: fn(),
          },
        ]}
      />
    </div>
  )
}

const meta = {
  title: 'UI/CodeEditor',
  component: RevealOnMount,
  parameters: { layout: 'padded' },
} satisfies Meta<typeof RevealOnMount>

export default meta
type Story = StoryObj<typeof meta>

export const RevealLineOnMount: Story = {
  name: 'reveal line 300 on mount',
  args: { line: 300 },
}

export const RevealLinesOnMount: StoryObj = {
  name: 'reveal lines 4–9 on mount (selected)',
  render: () => <RevealLinesDemo from={4} to={9} />,
}

export const WithSelectionActions: StoryObj = {
  name: 'selection actions (select text)',
  render: () => <SelectionActions />,
}
