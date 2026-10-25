import { Check, ChevronRight, Copy, Search } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Input } from '@/components/shadcn/input'
import { EmptyState } from '@/components/ui/EmptyState'
import { JsonHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import { redactAttributeEntries } from '../lib/redactAttributes'
import type { VisualizationSpan } from '../lib/traceTransform'
import { useCopyToClipboard } from '../lib/traceUtils'

interface SpanTagsTabProps {
  span: VisualizationSpan
  /**
   * The span's function-trigger redactor (`spanRawRedactor` in
   * functionTriggerFromSpan.ts), when the info tab's card has one. Attribute
   * values (`tool.arguments` among them) run through it before they render
   * OR are copied — the identical payload the info tab's card already hides
   * is one tab-click away here otherwise.
   */
  redact?: (value: unknown) => unknown
}

/**
 * The exact text one attribute row's copy button puts on the clipboard.
 * Pinned as its own function so the redaction that already ran (see
 * `entries` below) is provably what reaches the clipboard, without needing
 * to simulate a click — this tab's tests stay jsdom-free.
 */
export function attributeCopyText(key: string, value: unknown): string {
  return `${key}: ${typeof value === 'object' ? JSON.stringify(value) : String(value)}`
}

const NAMESPACE_LABELS: Record<string, string> = {
  gen_ai: 'llm',
  http: 'http',
  db: 'database',
  rpc: 'rpc',
  net: 'network',
  messaging: 'messaging',
  faas: 'faas',
  enduser: 'end user',
  thread: 'thread',
  code: 'code',
  otel: 'opentelemetry',
  service: 'worker',
  telemetry: 'telemetry',
  process: 'process',
  os: 'os',
  host: 'host',
  container: 'container',
  k8s: 'kubernetes',
  cloud: 'cloud',
  deployment: 'deployment',
}

interface AttributeGroup {
  namespace: string
  label: string
  entries: [string, unknown][]
}

export function SpanTagsTab({ span, redact }: SpanTagsTabProps) {
  const [searchQuery, setSearchQuery] = useState('')
  const { copiedKey, copy } = useCopyToClipboard()
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set())

  const attributes = span.attributes || {}
  // Redacted ONCE here — every render below and the copy button both read
  // from this, so they cannot disagree about what is safe to show.
  const entries = useMemo(
    () => redactAttributeEntries(attributes, redact),
    [attributes, redact],
  )

  const filteredEntries = useMemo(() => {
    return entries.filter(([key, value]) => {
      if (!searchQuery) return true
      const query = searchQuery.toLowerCase()
      return (
        key.toLowerCase().includes(query) ||
        String(value).toLowerCase().includes(query)
      )
    })
  }, [entries, searchQuery])

  const groups = useMemo(() => {
    const grouped = new Map<string, [string, unknown][]>()

    for (const [key, value] of filteredEntries) {
      const dotIndex = key.indexOf('.')
      const namespace = dotIndex > 0 ? key.substring(0, dotIndex) : '_other'

      if (!grouped.has(namespace)) {
        grouped.set(namespace, [])
      }
      grouped.get(namespace)?.push([key, value])
    }

    const result: AttributeGroup[] = []
    const sortedNamespaces = Array.from(grouped.keys()).sort((a, b) => {
      if (a === '_other') return 1
      if (b === '_other') return -1
      return a.localeCompare(b)
    })

    for (const ns of sortedNamespaces) {
      result.push({
        namespace: ns,
        label: ns === '_other' ? 'other' : NAMESPACE_LABELS[ns] || ns,
        entries: grouped.get(ns) ?? [],
      })
    }

    return result
  }, [filteredEntries])

  const copyToClipboard = (key: string, value: unknown) => {
    copy(key, attributeCopyText(key, value))
  }

  const toggleGroup = (namespace: string) => {
    setCollapsedGroups((prev) => {
      const next = new Set(prev)
      if (next.has(namespace)) {
        next.delete(namespace)
      } else {
        next.add(namespace)
      }
      return next
    })
  }

  if (entries.length === 0) {
    return (
      <div className="p-4">
        <EmptyState
          icon={Search}
          title="no attributes"
          description="this span has no recorded attributes"
        />
      </div>
    )
  }

  const showGrouped = groups.length > 1 && !searchQuery

  return (
    <div className="flex flex-col gap-3 p-4 font-inter">
      <div className="relative">
        <Search
          className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground"
          aria-hidden
        />
        <Input
          type="search"
          placeholder="Filter attributes"
          aria-label="filter attributes"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          className="h-8 rounded-md bg-muted/50 pl-8 text-sm [&::-webkit-search-cancel-button]:hidden"
        />
      </div>

      {showGrouped ? (
        groups.map((group) => {
          const isCollapsed = collapsedGroups.has(group.namespace)

          return (
            <div
              key={group.namespace}
              className="overflow-hidden rounded-lg border border-border"
            >
              <button
                type="button"
                onClick={() => toggleGroup(group.namespace)}
                aria-expanded={!isCollapsed}
                className="flex h-8 w-full items-center gap-2 px-3 text-left transition-colors hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50"
              >
                <ChevronRight
                  className={cn(
                    'size-4 text-muted-foreground transition-transform duration-[var(--motion-duration-control)]',
                    !isCollapsed && 'rotate-90',
                  )}
                  aria-hidden
                />
                <span className="text-xs font-semibold text-foreground">
                  {group.label}
                </span>
                <span className="ml-auto font-mono text-[11px] text-muted-foreground tabular-nums">
                  {group.entries.length}
                </span>
              </button>

              {!isCollapsed && (
                <div className="divide-y divide-border border-t border-border">
                  {group.entries.map(([key, value]) => (
                    <AttributeRow
                      key={key}
                      attrKey={key}
                      value={value}
                      copiedKey={copiedKey}
                      onCopy={copyToClipboard}
                    />
                  ))}
                </div>
              )}
            </div>
          )
        })
      ) : (
        <div className="divide-y divide-border overflow-hidden rounded-lg border border-border">
          {filteredEntries.map(([key, value]) => (
            <AttributeRow
              key={key}
              attrKey={key}
              value={value}
              copiedKey={copiedKey}
              onCopy={copyToClipboard}
            />
          ))}
        </div>
      )}

      {filteredEntries.length === 0 && searchQuery && (
        <div className="py-6 text-center">
          <p className="text-[13px] text-muted-foreground">
            No attributes match &ldquo;{searchQuery}&rdquo;
          </p>
        </div>
      )}

      <div className="pt-1 text-center font-mono text-[11px] text-muted-foreground tabular-nums">
        {filteredEntries.length} of {entries.length} attributes
      </div>
    </div>
  )
}

interface AttributeRowProps {
  attrKey: string
  value: unknown
  copiedKey: string | null
  onCopy: (key: string, value: unknown) => void
}

function AttributeRow({
  attrKey,
  value,
  copiedKey,
  onCopy,
}: AttributeRowProps) {
  const isObject = typeof value === 'object' && value !== null
  const formatted = isObject ? JSON.stringify(value, null, 2) : String(value)
  const isCopied = copiedKey === attrKey

  return (
    <button
      type="button"
      onClick={() => onCopy(attrKey, value)}
      aria-label={`copy ${attrKey}`}
      className="group/attr w-full px-3 py-1.5 text-left transition-colors hover:bg-muted/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="mb-0.5 truncate font-mono text-[11px] text-muted-foreground">
            {attrKey}
          </div>
          {isObject ? (
            <div className="code-theme-editor overflow-hidden rounded-md border border-border">
              <JsonHighlight code={formatted} wrap className="px-3 py-2" />
            </div>
          ) : (
            <div className="font-mono text-xs leading-relaxed break-all text-foreground tabular-nums">
              {formatted}
            </div>
          )}
        </div>
        {isCopied ? (
          <Check className="mt-0.5 size-4 shrink-0 text-ok" aria-hidden />
        ) : (
          <Copy
            className="mt-0.5 size-4 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover/attr:opacity-100 group-focus-visible/attr:opacity-100"
            aria-hidden
          />
        )}
      </div>
    </button>
  )
}
