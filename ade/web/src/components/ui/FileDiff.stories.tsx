import type { Meta, StoryObj } from '@storybook/react-vite'
import { FileDiff } from './FileDiff'

const OLD = `use std::fs;

fn main() {
    let cfg = Config::load("iii.toml");
    println!("starting on {}", cfg.port);
    run(cfg);
}
`

const NEW = `use std::fs;
use tracing::info;

fn main() {
    let cfg = Config::load("iii.toml").expect("config");
    info!(port = cfg.port, "starting");
    run(&cfg);
}
`

const meta = {
  title: 'UI/FileDiff',
  component: FileDiff,
  parameters: { layout: 'padded' },
  args: {
    oldFile: { name: 'src/main.rs', contents: OLD },
    newFile: { name: 'src/main.rs', contents: NEW },
  },
  argTypes: {
    diffStyle: { control: 'select', options: ['unified', 'split'] },
    lineDiffType: {
      control: 'select',
      options: ['word-alt', 'word', 'char', 'none'],
    },
  },
  decorators: [
    (Story) => (
      <div className="max-w-4xl">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof FileDiff>

export default meta
type Story = StoryObj<typeof meta>

export const Unified: Story = {}

export const Split: Story = { args: { diffStyle: 'split' } }

/** Empty old contents render a created file; the mirror image is a deletion. */
export const Created: Story = {
  args: { oldFile: { name: 'src/main.rs', contents: '' } },
}

export const Collapsed: Story = { args: { collapsed: true } }

/** Callers supplying their own review row hide Pierre's file header. */
export const NoHeader: Story = { args: { disableFileHeader: true } }
