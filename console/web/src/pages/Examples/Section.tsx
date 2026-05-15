import type * as React from 'react'
import { Prompt } from '@/components/ui/Prompt'
import { cn } from '@/lib/utils'

interface SectionProps {
  id?: string
  eyebrow: string
  title: string
  description?: React.ReactNode
  children: React.ReactNode
  className?: string
}

export function Section({
  id,
  eyebrow,
  title,
  description,
  children,
  className,
}: SectionProps) {
  return (
    <section id={id} className={cn('scroll-mt-20', className)}>
      <header className="mb-6">
        <div className="font-mono text-[11px] uppercase tracking-[0.18em] text-ink-faint mb-2">
          <Prompt symbol="$">{eyebrow}</Prompt>
        </div>
        <h2 className="font-mono text-[20px] font-medium tracking-[-0.02em] text-ink lowercase">
          {title}
        </h2>
        {description ? (
          <p className="mt-2 font-mono text-[13px] leading-[1.7] text-ink-faint max-w-[60ch] lowercase">
            {description}
          </p>
        ) : null}
      </header>
      <div>{children}</div>
    </section>
  )
}

interface VariantCardProps {
  label: string
  children: React.ReactNode
  className?: string
}

export function VariantCard({ label, children, className }: VariantCardProps) {
  return (
    <div className={cn('border border-rule bg-bg', className)}>
      <div className="bg-panel px-3 py-1.5 border-b border-rule-2 font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
        {label}
      </div>
      <div className="p-4">{children}</div>
    </div>
  )
}
