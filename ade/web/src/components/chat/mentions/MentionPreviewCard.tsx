import { ArrowUpRight } from 'lucide-react'
import { ErrorBoundary } from '@/components/ui/ErrorBoundary'
import {
  factLabelClassName,
  factValueClassName,
} from '@/components/ui/FactCard'
import { useMentionProviders } from '@/lib/mentions/providers'
import type { MentionProvider, MentionView } from '@/lib/mentions/types'
import { useMentionView } from '@/lib/mentions/views'
import { ExtensionScopeProvider } from '@/lib/ui-scope'
import { useExtMentionRenderer } from '@/lib/ui-slots'
import { cn } from '@/lib/utils'
import {
  MentionGlyph,
  mentionColor,
  mentionIcon,
  mentionToneClass,
} from './appearance'
import { useOpenMention } from './open-mention'

interface MentionPreviewCardProps {
  name: string
  id: string
}

const cardShell =
  'mention-card min-w-0 rounded-lg bg-panel-raised px-3 py-2.5 font-inter text-sm shadow-raised ring-1 ring-foreground/10'

/**
 * The preview of one mention, shown when its pill is hovered: the worker's
 * own renderer when it registered one (`host.mentions.registerRenderer`),
 * the generic card otherwise — and the generic card again if that renderer
 * throws.
 */
export function MentionPreviewCard({ name, id }: MentionPreviewCardProps) {
  const state = useMentionView(name, id)
  const provider = useMentionProviders().byName.get(name)
  const renderer = useExtMentionRenderer(name)
  const open = useOpenMention()

  if (state.status === 'loading') {
    return (
      <div
        className={cn(cardShell, 'flex items-center gap-2')}
        data-color="neutral"
      >
        <span className="h-3 w-24 animate-pulse rounded-xs bg-surface" />
        <span className="h-3 w-40 animate-pulse rounded-xs bg-surface" />
      </div>
    )
  }
  if (state.status !== 'ready') {
    const message =
      state.status === 'missing'
        ? `${provider?.label ?? name}: this item no longer exists`
        : state.status === 'unknown-provider'
          ? `No installed worker provides @${name} mentions`
          : `Could not load @${name}: ${state.message}`
    return (
      <div className={cn(cardShell, 'text-ink-faint')} data-color="neutral">
        {message}
      </div>
    )
  }

  const view = state.view
  const onOpen = () => {
    open(view.open)
  }
  const generic = (
    <GenericMentionCard
      provider={provider}
      name={name}
      view={view}
      onOpen={view.open ? onOpen : undefined}
    />
  )
  if (!renderer) return generic
  const Preview = renderer.Preview
  return (
    <ExtensionScopeProvider scope={renderer.scope}>
      <div data-iii-ui={renderer.scope} style={{ display: 'contents' }}>
        <ErrorBoundary fallback={() => generic}>
          <Preview provider={name} view={view} open={onOpen} />
        </ErrorBoundary>
      </div>
    </ExtensionScopeProvider>
  )
}

interface GenericMentionCardProps {
  provider?: MentionProvider
  name: string
  view: MentionView
  onOpen?: () => void
}

/** Fields the generic card shows at most. */
const CARD_FIELDS = 6

/** Icon, provider and handle on top; the name; a line of context; fields. */
export function GenericMentionCard({
  provider,
  name,
  view,
  onOpen,
}: GenericMentionCardProps) {
  const color = mentionColor(view.color, provider?.color)
  const icon = mentionIcon(view.icon, provider?.icon)
  const fields = (view.fields ?? []).slice(0, CARD_FIELDS)
  return (
    <div className={cardShell} data-color={color} data-mention-card={name}>
      <div className="flex min-w-0 items-center gap-1.5 text-xs text-ink-faint">
        <span
          className="mention-chip flex size-6 shrink-0 items-center justify-center rounded-xs"
          data-color={color}
        >
          <MentionGlyph icon={icon} color={color} className="size-4" />
        </span>
        <span className="truncate">{provider?.label ?? name}</span>
        {view.hint ? (
          <span className="shrink-0 font-mono text-xs text-ink-faint">
            {view.hint}
          </span>
        ) : null}
        {onOpen ? (
          <button
            type="button"
            onClick={onOpen}
            className="ml-auto inline-flex shrink-0 items-center gap-0.5 rounded-xs px-1 text-xs text-ink-faint hover:bg-surface-hover hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rule-focus"
          >
            open
            <ArrowUpRight aria-hidden className="size-4" />
          </button>
        ) : null}
      </div>
      {onOpen ? (
        <button
          type="button"
          onClick={onOpen}
          className="mt-1 block max-w-full truncate text-left text-sm font-medium text-ink hover:underline focus-visible:outline-none focus-visible:underline"
        >
          {view.label}
        </button>
      ) : (
        <div className="mt-1 truncate text-sm font-medium text-ink">
          {view.label}
        </div>
      )}
      {view.description ? (
        <div className="mt-0.5 line-clamp-2 text-xs text-ink-faint">
          {view.description}
        </div>
      ) : null}
      {fields.length > 0 ? (
        <dl className="mt-2 grid grid-cols-[repeat(auto-fill,minmax(7.5rem,1fr))] gap-x-3 gap-y-1.5">
          {fields.map((field) => (
            <div key={field.label} className="min-w-0">
              <dt className={cn(factLabelClassName, 'truncate text-xs')}>
                {field.label}
              </dt>
              <dd
                className={cn(
                  factValueClassName,
                  'truncate',
                  mentionToneClass(field.tone),
                )}
              >
                {field.value}
              </dd>
            </div>
          ))}
        </dl>
      ) : null}
    </div>
  )
}
