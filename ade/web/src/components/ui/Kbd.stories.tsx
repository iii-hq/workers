import type { Meta, StoryObj } from '@storybook/react-vite'
import type { ReactNode } from 'react'
import { Kbd } from './Kbd'
import { KeyCombo } from './KeyCombo'

const meta = {
  title: 'UI/Kbd',
  component: Kbd,
} satisfies Meta<typeof Kbd>

export default meta
type Story = StoryObj<typeof meta>

function Surface({
  label,
  className,
  children,
}: {
  label: string
  className: string
  children: ReactNode
}) {
  return (
    <div className={`flex flex-col gap-3 rounded-sm p-4 ${className}`}>
      <span className="font-sans text-[11px] text-ink-faint">{label}</span>
      <div className="flex flex-wrap items-center gap-4">{children}</div>
    </div>
  )
}

/** Caps and chords on each base layer: the keycap shadow is composed from
    the lift ingredients, so it re-tints with the theme. */
export const Caps: Story = {
  render: () => (
    <div className="flex w-[36rem] flex-col gap-4">
      {[
        ['bg', 'bg-bg'],
        ['panel', 'bg-panel'],
        ['panel-raised', 'bg-panel-raised'],
      ].map(([label, className]) => (
        <Surface key={label} label={label} className={className}>
          <KeyCombo binding="Mod+K" platform="mac" />
          <KeyCombo binding="Ctrl+<" platform="mac" />
          <KeyCombo binding="Ctrl+>" platform="mac" />
          <KeyCombo binding="Ctrl+G C" platform="mac" />
          <KeyCombo binding="Alt+>" platform="other" />
          <KeyCombo binding="Mod+K" platform="other" />
          <Kbd>esc</Kbd>
          <Kbd>↵</Kbd>
        </Surface>
      ))}
      <p className="font-sans text-[13px] text-ink">
        In prose a single cap names the key: press <Kbd>esc</Kbd> to close the
        preview, <Kbd>↵</Kbd> to open it.
      </p>
    </div>
  ),
}
