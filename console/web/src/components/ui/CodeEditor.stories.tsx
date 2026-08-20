import type { Meta, StoryObj } from '@storybook/react-vite'
import { useEffect, useRef, useState } from 'react'
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
