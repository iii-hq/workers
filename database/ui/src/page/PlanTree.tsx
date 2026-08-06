/**
 * Query plan, as a tree rather than a wall of text.
 *
 * `handlers/explain.rs` already normalised three dialects into one shape and
 * computed the warnings, so this only draws. The one visual that carries most
 * of the value is the per-node cost bar: it shows you *where* the time went
 * without reading a single number, which is the difference between a plan you
 * skim and one you decipher.
 */

import { Badge, Tooltip, TooltipContent, TooltipTrigger } from '@iii-dev/console-ui'
import { useState } from 'react'
import type { ExplainResult, PlanNode } from '../lib/rpc'
import { AlertCircle, ChevronRight } from './icons'

interface WarningsByNode {
  [nodeId: number]: { kind: string; message: string; severity: string }[]
}

export function PlanTree({ plan }: { plan: ExplainResult }) {
  const [collapsed, setCollapsed] = useState<ReadonlySet<number>>(new Set())

  if (!plan.root) {
    return <p className="db-msg">the driver returned no plan for this statement.</p>
  }

  const byNode: WarningsByNode = {}
  for (const w of plan.warnings) {
    ;(byNode[w.node_id] ??= []).push(w)
  }

  const rootCost = plan.root.cost_total ?? 0

  const toggle = (id: number) =>
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })

  return (
    <div className="db-plan">
      <div className="db-plan-bar">
        <span className="db-plan-format">{plan.format}</span>
        {plan.analyzed ? <Badge variant="accent">measured</Badge> : <Badge variant="default">estimated</Badge>}
        {plan.warnings.length > 0 ? (
          <span className="db-plan-warncount">
            {plan.warnings.length} warning{plan.warnings.length === 1 ? '' : 's'}
          </span>
        ) : null}
      </div>
      {/* A plain nested list, not `role="tree"`. The full tree pattern needs
          roving tabindex and arrow-key traversal; claiming the role without
          them announces navigation to a screen reader that does not work.
          The disclosure buttons are focusable and labelled, which is the
          honest subset. */}
      <ul className="db-plan-tree">
        <Node
          node={plan.root}
          depth={0}
          rootCost={rootCost}
          warnings={byNode}
          collapsed={collapsed}
          onToggle={toggle}
        />
      </ul>
    </div>
  )
}

function Node({
  node,
  depth,
  rootCost,
  warnings,
  collapsed,
  onToggle,
}: {
  node: PlanNode
  depth: number
  rootCost: number
  warnings: WarningsByNode
  collapsed: ReadonlySet<number>
  onToggle: (id: number) => void
}) {
  const mine = warnings[node.id] ?? []
  const isCollapsed = collapsed.has(node.id)
  const hasChildren = node.children.length > 0
  const share = rootCost > 0 ? Math.min(1, (node.cost_total ?? 0) / rootCost) : 0
  const worst = mine.some((w) => w.severity === 'high') ? 'high' : mine.length ? 'warn' : null

  return (
    <li className="db-plan-li">
      <div className={`db-plan-node${worst ? ` flag-${worst}` : ''}`} style={{ paddingLeft: `${depth * 14 + 4}px` }}>
        <button
          type="button"
          className={`db-plan-twist${hasChildren ? '' : ' leaf'}${isCollapsed ? '' : ' open'}`}
          onClick={() => hasChildren && onToggle(node.id)}
          aria-label={isCollapsed ? 'expand' : 'collapse'}
          aria-expanded={hasChildren ? !isCollapsed : undefined}
          tabIndex={hasChildren ? 0 : -1}
        >
          {hasChildren ? <ChevronRight size={11} aria-hidden /> : null}
        </button>

        <span className="db-plan-class">{node.node_class}</span>
        <span className="db-plan-label">{node.label}</span>
        {node.relation ? <span className="db-plan-rel">on {node.relation}</span> : null}

        {mine.map((w) => (
          <Tooltip key={w.kind}>
            <TooltipTrigger asChild>
              <span className="db-plan-warn">
                <AlertCircle size={12} aria-label={w.kind} />
              </span>
            </TooltipTrigger>
            <TooltipContent side="top">{w.message}</TooltipContent>
          </Tooltip>
        ))}

        <span className="db-plan-spacer" />

        <span className="db-plan-metrics">
          {node.rows_actual != null && node.rows_estimated != null ? (
            <Metric
              label="rows"
              value={`${fmt(node.rows_actual)}/${fmt(node.rows_estimated)}`}
              title="actual / estimated"
            />
          ) : node.rows_estimated != null ? (
            <Metric label="rows" value={fmt(node.rows_estimated)} title="estimated" />
          ) : null}
          {node.loops != null && node.loops > 1 ? <Metric label="loops" value={fmt(node.loops)} /> : null}
          {node.time_ms != null ? <Metric label="time" value={`${node.time_ms.toFixed(2)}ms`} /> : null}
          {node.cost_total != null ? <Metric label="cost" value={fmt(node.cost_total)} /> : null}
        </span>

        <span className="db-track plan" title={`${(share * 100).toFixed(1)}% of total cost`}>
          <span className="db-track-fill" style={{ width: `${share * 100}%` }} />
        </span>
      </div>

      {node.detail ? (
        <div className="db-plan-detail" style={{ paddingLeft: `${depth * 14 + 26}px` }}>
          {node.detail}
        </div>
      ) : null}

      {hasChildren && !isCollapsed ? (
        <ul>
          {node.children.map((c) => (
            <Node
              key={c.id}
              node={c}
              depth={depth + 1}
              rootCost={rootCost}
              warnings={warnings}
              collapsed={collapsed}
              onToggle={onToggle}
            />
          ))}
        </ul>
      ) : null}
    </li>
  )
}

function Metric({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <span className="db-plan-metric" title={title}>
      <span className="db-plan-metric-k">{label}</span>
      <span className="db-num">{value}</span>
    </span>
  )
}

function fmt(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}m`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`
  return Number.isInteger(n) ? String(n) : n.toFixed(2)
}
