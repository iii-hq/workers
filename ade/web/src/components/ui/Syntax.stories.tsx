import type { Meta, StoryObj } from '@storybook/react-vite'
import { CodeHighlight, JsonHighlight } from '@/lib/syntax'

const meta = {
  title: 'UI/Syntax',
  parameters: { layout: 'padded' },
  decorators: [
    (Story) => (
      <div className="max-w-2xl">
        <Story />
      </div>
    ),
  ],
} satisfies Meta

export default meta
type Story = StoryObj

const RUST = `use tracing::info;

/// Boot the worker and block on its event loop.
pub async fn main() -> anyhow::Result<()> {
    let cfg = Config::load("iii.toml")?;
    info!(port = cfg.port, "starting");
    run(&cfg).await
}
`

const BASH = `#!/usr/bin/env bash
set -euo pipefail
iii trigger browser::open --url "https://example.com/a/very/long/path/that/keeps/going"
iii trigger compose::restart | tee -a logs/restart.out
`

const payload = {
  function_id: 'browser::open',
  args: { url: 'https://example.com', incognito: false, timeout_ms: 30000 },
  tags: ['worker', 'browser'],
  result: null,
}

export const Rust: Story = {
  render: () => <CodeHighlight language="rust" code={RUST} />,
}

export const Shell: Story = {
  render: () => <CodeHighlight language="bash" code={BASH} />,
}

export const Json: Story = {
  render: () => <JsonHighlight code={JSON.stringify(payload, null, 2)} />,
}

/** Unregistered grammars keep the chrome and drop the coloring. */
export const UnknownLanguage: Story = {
  render: () => <CodeHighlight language="brainfuck" code={BASH} />,
}

/** `wrap` trades strict columns for soft-wrapped long lines in narrow panes. */
export const Wrapped: Story = {
  render: () => (
    <div className="flex w-80 flex-col gap-3">
      <CodeHighlight language="bash" code={BASH} wrap />
      <JsonHighlight code={JSON.stringify(payload)} wrap />
    </div>
  ),
}
