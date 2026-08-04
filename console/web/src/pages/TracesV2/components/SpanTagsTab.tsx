import { ChevronRight, Copy, Search } from 'lucide-react'
import { useMemo, useState } from 'react'
import { EmptyState } from '@/components/ui/EmptyState'
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
 * to simulate a click — console/web's tests stay jsdom-free.
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
    <div className="p-5 space-y-3">
      <div className="relative">
        <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-ink-faint" />
        <input
          type="text"
          placeholder="filter attributes..."
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          className="w-full pl-9 pr-4 py-2 bg-bg border border-rule font-mono text-[13px] text-ink placeholder-ink-ghost lowercase focus:outline-none focus:border-accent transition-colors"
        />
      </div>

      {showGrouped ? (
        groups.map((group) => {
          const isCollapsed = collapsedGroups.has(group.namespace)

          return (
            <div key={group.namespace} className="border border-rule bg-bg">
              <button
                type="button"
                onClick={() => toggleGroup(group.namespace)}
                className="w-full flex items-center gap-2 px-4 py-2.5 hover:bg-panel transition-colors text-left"
              >
                <ChevronRight
                  className={`w-3 h-3 text-ink-faint transition-transform ${isCollapsed ? '' : 'rotate-90'}`}
                />
                <span className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint">
                  {group.label}
                </span>
                <span className="font-mono text-[10px] text-ink-ghost ml-auto tabular-nums">
                  {group.entries.length}
                </span>
              </button>

              {!isCollapsed && (
                <div className="border-t border-rule divide-y divide-rule">
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
        <div className="border border-rule bg-bg divide-y divide-rule">
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
        <div className="text-center py-6">
          <p className="font-mono text-[13px] text-ink-faint lowercase">
            no attributes match &ldquo;{searchQuery}&rdquo;
          </p>
        </div>
      )}

      <div className="font-mono text-[10px] text-ink-ghost text-center pt-2 lowercase tabular-nums">
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
      className="w-full px-4 py-2 hover:bg-panel transition-colors text-left group"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1 min-w-0">
          <div className="font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint mb-0.5">
            {attrKey}
          </div>
          {isObject ? (
            <pre className="border border-rule bg-bg px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink overflow-x-auto whitespace-pre">
              {formatted}
            </pre>
          ) : (
            <div className="font-mono text-[12px] text-ink break-all leading-relaxed tabular-nums">
              {formatted}
            </div>
          )}
        </div>
        {isCopied ? (
          <span className="font-mono text-[10px] uppercase tracking-[0.06em] text-accent flex-shrink-0 mt-0.5">
            copied
          </span>
        ) : (
          <Copy className="w-3 h-3 text-ink-ghost opacity-0 group-hover:opacity-100 transition-opacity flex-shrink-0 mt-0.5" />
        )}
      </div>
    </button>
  )
}
