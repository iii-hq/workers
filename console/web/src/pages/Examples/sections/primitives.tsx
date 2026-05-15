import { useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Caret } from '@/components/ui/Caret'
import { ModeToggle } from '@/components/ui/ModeToggle'
import { Prompt } from '@/components/ui/Prompt'
import { Select } from '@/components/ui/Select'
import { StatusDot } from '@/components/ui/StatusDot'
import { Wordmark } from '@/components/ui/Wordmark'
import { Section, VariantCard } from '../Section'

type Pick = 'a' | 'b' | 'c'

export function PrimitivesSection() {
  const [toggle2, setToggle2] = useState<'on' | 'off'>('on')
  const [toggle3, setToggle3] = useState<Pick>('a')
  const [select, setSelect] = useState<'one' | 'two' | 'three'>('one')

  return (
    <Section
      id="primitives"
      eyebrow="primitives"
      title="ui primitives"
      description="every reusable building block from the schematic."
    >
      <div className="grid grid-cols-1 @3xl:grid-cols-2 gap-6">
        <VariantCard label="button variants" className="@3xl:col-span-2">
          <div className="flex flex-wrap gap-3 items-center">
            <Button variant="primary">primary</Button>
            <Button variant="ghost">ghost</Button>
            <Button variant="pill" size="sm">
              pill
            </Button>
            <Button variant="icon" size="icon" aria-label="settings">
              <svg
                width="14"
                height="14"
                viewBox="0 0 14 14"
                fill="none"
                stroke="currentColor"
                strokeWidth="1"
                aria-hidden="true"
              >
                <path d="M2 4H12M2 7H12M2 10H12" />
              </svg>
            </Button>
            <Button variant="terminal">
              <Prompt symbol="$" /> npm install
            </Button>
            <Button variant="wiggle">wiggle</Button>
            <Button variant="primary" disabled>
              disabled
            </Button>
          </div>
        </VariantCard>

        <VariantCard label="button sizes">
          <div className="flex flex-wrap gap-3 items-center">
            <Button size="sm">sm</Button>
            <Button size="md">md</Button>
            <Button size="lg">lg</Button>
          </div>
        </VariantCard>

        <VariantCard label="mode toggle">
          <div className="flex flex-col gap-3 items-start">
            <ModeToggle<'on' | 'off'>
              value={toggle2}
              onChange={setToggle2}
              options={[
                { value: 'on', label: 'on' },
                { value: 'off', label: 'off' },
              ]}
            />
            <ModeToggle<Pick>
              value={toggle3}
              onChange={setToggle3}
              options={[
                { value: 'a', label: 'one' },
                { value: 'b', label: 'two' },
                { value: 'c', label: 'three' },
              ]}
            />
          </div>
        </VariantCard>

        <VariantCard label="select">
          <Select<'one' | 'two' | 'three'>
            value={select}
            onChange={setSelect}
            options={[
              { value: 'one', label: 'option one' },
              { value: 'two', label: 'option two' },
              { value: 'three', label: 'option three' },
            ]}
          />
        </VariantCard>

        <VariantCard label="prompt + caret + status dot">
          <div className="flex flex-col gap-3 font-mono text-[13px] text-ink">
            <span>
              <Prompt symbol="$">install the latest release</Prompt>
            </span>
            <span>
              <Prompt symbol=">" /> typing
              <Caret className="ml-1" />
            </span>
            <span className="flex items-center gap-3 text-[12px] text-ink-faint">
              <StatusDot tone="accent" pulse /> live
              <StatusDot tone="alert" /> alert
              <StatusDot tone="warn" /> warn
              <StatusDot tone="ink" /> idle
            </span>
          </div>
        </VariantCard>

        <VariantCard label="wordmark">
          <div className="flex items-center gap-4">
            <Wordmark />
            <span className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint">
              three eye
            </span>
          </div>
        </VariantCard>
      </div>
    </Section>
  )
}
