import type { Meta, StoryObj } from '@storybook/react-vite'
import { useState } from 'react'
import { CodeEditor } from './CodeEditor'
import { MarkdownPreview } from './MarkdownPreview'

const README = `# browser

Drives a shared Chromium so agents can open tabs, read pages and stream
screencasts.

## Usage

\`\`\`bash
iii trigger browser::open --url https://example.com
\`\`\`

- Tabs are cheap; sessions own cookies and storage.
- Idle tabs **sleep** and wake on the next call.

> Screencast frames arrive in CSS pixels, not device pixels.

| Function | Purpose |
| --- | --- |
| \`browser::open\` | Open a tab |
| \`browser::screenshot\` | Capture the viewport |
`

const meta = {
  title: 'UI/MarkdownPreview',
  component: MarkdownPreview,
  parameters: { layout: 'padded' },
  args: { markdown: README },
} satisfies Meta<typeof MarkdownPreview>

export default meta
type Story = StoryObj<typeof meta>

export const Default: Story = {
  decorators: [
    (Story) => (
      <div className="max-w-2xl">
        <Story />
      </div>
    ),
  ],
}

/** The intended pairing: raw markdown in CodeEditor, rendered on the right. */
function Editor({ markdown }: { markdown: string }) {
  const [value, setValue] = useState(markdown)
  return (
    <div className="grid h-[420px] grid-cols-2 gap-px bg-edge">
      <div className="overflow-auto bg-panel">
        <CodeEditor
          value={value}
          onChange={setValue}
          language="markdown"
          aria-label="Markdown source"
        />
      </div>
      <MarkdownPreview markdown={value} className="overflow-auto" />
    </div>
  )
}

export const SideBySide: Story = {
  render: (args) => <Editor markdown={args.markdown} />,
}
