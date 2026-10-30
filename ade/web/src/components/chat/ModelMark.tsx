import { useId } from 'react'
import { cn } from '@/lib/utils'
import type { ModelId } from '@/types/chat'
import { type ModelFamily, modelFamily } from './model-picker-presentation'
import { ProviderIcon } from './ProviderIcon'

interface ModelMarkProps {
  modelId: ModelId | null | undefined
  /** The model's catalog label, read alongside the id for the family. */
  modelLabel?: string | null
  /** The serving provider, for the fallback mark. */
  providerId?: string | null
  providerLabel: string
  providerIconSvg?: string | null
  /** Size classes; defaults to the shared 16px icon baseline. */
  className?: string
}

/**
 * A model's mark: its family's (the Claude Code mascot for Claude models,
 * the Codex cloud for GPT ones) whichever provider serves it, so one model
 * reads the same through an API key or a subscription. A model outside a
 * known family keeps its provider's mark.
 */
export function ModelMark({
  modelId,
  modelLabel,
  providerId,
  providerLabel,
  providerIconSvg,
  className,
}: ModelMarkProps) {
  const family = modelFamily(modelId, modelLabel)
  if (family) return <FamilyMark family={family} className={className} />
  return (
    <ProviderIcon
      iconSvg={providerIconSvg}
      label={providerLabel}
      providerId={providerId}
      className={className}
    />
  )
}

function FamilyMark({
  family,
  className,
}: {
  family: ModelFamily
  className?: string
}) {
  // `url(#…)` wants a plain fragment; React's ids carry punctuation.
  const gradientId = `codex-${useId().replace(/[^\w-]/g, '')}`
  return family === 'claude' ? (
    <svg
      aria-hidden="true"
      data-model-mark="claude"
      viewBox="0 0 22 14"
      className={cn('size-4 shrink-0', className)}
    >
      {/* Body with its two eyes cut out, the arms, then four legs. */}
      <path
        fill="#d97757"
        fillRule="evenodd"
        d="M3 0h16v10H3zM6 3v3h2V3zm8 0v3h2V3zM0 4h3v3H0zm19 0h3v3h-3zM4 10h2v4H4zm3 0h2v4H7zm6 0h2v4h-2zm3 0h2v4h-2z"
      />
    </svg>
  ) : (
    <svg
      aria-hidden="true"
      data-model-mark="gpt"
      viewBox="0 0 24 24"
      className={cn('size-4 shrink-0', className)}
    >
      <defs>
        <linearGradient
          id={gradientId}
          x1="3"
          y1="2"
          x2="21"
          y2="22"
          gradientUnits="userSpaceOnUse"
        >
          <stop offset="0" stopColor="#a5b4ff" />
          <stop offset="0.55" stopColor="#6d7bff" />
          <stop offset="1" stopColor="#5146f0" />
        </linearGradient>
      </defs>
      <g fill={`url(#${gradientId})`}>
        <circle cx="12" cy="12" r="7.5" />
        <circle cx="7.5" cy="7.5" r="4.5" />
        <circle cx="13" cy="5.5" r="4.5" />
        <circle cx="18" cy="9" r="4.5" />
        <circle cx="18.5" cy="15" r="4.5" />
        <circle cx="13" cy="18.5" r="4.5" />
        <circle cx="7" cy="17" r="4.5" />
        <circle cx="5" cy="12" r="4.5" />
      </g>
      <path
        fill="none"
        stroke="#fff"
        strokeWidth="1.9"
        strokeLinecap="round"
        strokeLinejoin="round"
        d="M8.5 9.25 11.25 12 8.5 14.75M13 15h3.25"
      />
    </svg>
  )
}
