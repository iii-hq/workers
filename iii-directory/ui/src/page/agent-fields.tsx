/**
 * The agent profile form (see `agentsAdapter.customForm` in index.tsx):
 * a sectioned settings layout — Identity (avatar + name + description),
 * Behavior & capabilities (skill selection; the markdown editor below
 * the form is the system prompt), Execution (default model), and
 * Delegation. Everything edits the draft's frontmatter through the same
 * guarded `editDraft` the built-in fields use; empty selections remove
 * their key (absent `skills` = every skill, absent `delegates_to` =
 * every agent), matching the worker's semantics.
 */

import { Input, type Host } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { useEffect, useMemo, useState } from 'react'
import { type FormContext, slugify } from './browser'
import { TokenIcon } from './token-icons'
import {
  readFrontmatterField,
  readFrontmatterStringList,
  setFrontmatterField,
  setFrontmatterStringList,
  withoutFrontmatterFields,
} from './frontmatter'

interface PickItem {
  id: string
  label: string
  sublabel?: string
  desc?: string
  /** Tree-icon token rendered as the row glyph (agents). */
  iconToken?: string | null
}

/** One list-fn fetch per mounted collection; a failure leaves the picker
 * in its "edit in Content" fallback while still marking draft entries. */
function useCatalog(
  host: Host,
  fetch: (host: Host) => Promise<PickItem[]>,
): { items: PickItem[] | null; error: boolean } {
  const [items, setItems] = useState<PickItem[] | null>(null)
  const [error, setError] = useState(false)
  useEffect(() => {
    let cancelled = false
    fetch(host)
      .then((next) => {
        if (!cancelled) setItems(next)
      })
      .catch(() => {
        if (!cancelled) setError(true)
      })
    return () => {
      cancelled = true
    }
    // biome-ignore lint/correctness/useExhaustiveDependencies: fetch is a module-level constant per picker
  }, [host])
  return { items, error }
}

const fetchSkills = (host: Host) =>
  host.iii
    .trigger<{
      skills: { id: string; title: string; description: string }[]
    }>('directory::skills::list', { include_description: true })
    .then((out) =>
      (out.skills ?? []).map((s) => ({
        id: s.id,
        label: s.title && s.title !== s.id ? s.title : s.id,
        sublabel: s.title && s.title !== s.id ? s.id : undefined,
        desc: s.description || undefined,
      })),
    )

const fetchAgents = (host: Host) =>
  host.iii
    .trigger<{
      agents: {
        id: string
        name: string
        logo: string | null
        icon: string | null
        description: string
      }[]
    }>('directory::agents::list', {})
    .then((out) =>
      (out.agents ?? []).map((a) => ({
        id: a.id,
        label: a.name || a.id,
        sublabel: a.name && a.name !== a.id ? a.id : undefined,
        desc: a.description || undefined,
        iconToken: a.icon,
      })),
    )

const fetchModels = (host: Host) =>
  host.iii
    .trigger<{ models: { id: string }[] }>('router::models::list', {})
    .then((out) => (out.models ?? []).map((m) => ({ id: m.id, label: m.id })))

/** Curated avatar presets, one per harness `SubagentIcon` token — picking
 * one keeps the emoji↔icon mapping mechanical for spawn display
 * identities. */
const LOGO_PRESETS: { emoji: string; token: string }[] = [
  { emoji: '🤖', token: 'agent' },
  { emoji: '💻', token: 'code' },
  { emoji: '🔍', token: 'search' },
  { emoji: '📟', token: 'terminal' },
  { emoji: '💾', token: 'database' },
  { emoji: '🧪', token: 'test' },
  { emoji: '🧐', token: 'review' },
  { emoji: '📚', token: 'docs' },
  { emoji: '🎨', token: 'design' },
]

// ─────────────────────────── building blocks ─────────────────────────

function Section({
  title,
  hint,
  children,
}: {
  title: string
  hint: string
  children: ReactNode
}) {
  return (
    <section className="dir-ui-af-section">
      <h3 className="dir-ui-af-title">{title}</h3>
      <p className="dir-ui-af-hint">{hint}</p>
      <div className="dir-ui-af-card">{children}</div>
    </section>
  )
}

function Row({
  label,
  hint,
  children,
}: {
  label: string
  hint?: string
  children: ReactNode
}) {
  return (
    <div className="dir-ui-af-row">
      <span className="dir-ui-af-label">
        {label}
        {hint ? <span className="dir-ui-af-label-hint">{hint}</span> : null}
      </span>
      <div className="dir-ui-af-control">{children}</div>
    </div>
  )
}

/** Collapsed "N selected — click to edit" row expanding into a search +
 * checkbox list, the reference pattern for skills and delegates. */
function CollapsiblePicker({
  noun,
  emptyLabel,
  items,
  error,
  selected,
  missing,
  isChecked,
  toggle,
  readOnly,
}: {
  noun: string
  emptyLabel: string
  items: PickItem[] | null
  error: boolean
  selected: string[]
  missing: string[]
  isChecked: (id: string) => boolean
  toggle: (id: string) => void
  readOnly: boolean
}) {
  const [open, setOpen] = useState(false)
  const [filter, setFilter] = useState('')
  const needle = filter.trim().toLowerCase()
  const visible = (items ?? []).filter(
    (s) =>
      !needle ||
      s.id.toLowerCase().includes(needle) ||
      s.label.toLowerCase().includes(needle) ||
      (s.desc ?? '').toLowerCase().includes(needle),
  )
  const summary =
    selected.length === 0
      ? emptyLabel
      : `${selected.length} selected — click to edit${missing.length ? ` · ${missing.length} missing` : ''}`

  if (!open) {
    return (
      <button
        type="button"
        className="dir-ui-af-collapsed"
        onClick={() => setOpen(true)}
      >
        <span className="dir-ui-af-plus">＋</span>
        <span>{summary}</span>
        <span className="dir-ui-af-chev">⌄</span>
      </button>
    )
  }
  return (
    <div className="dir-ui-af-picker">
      <div className="dir-ui-af-picker-head">
        <span className="dir-ui-af-picker-count">
          {selected.length ? `${selected.length} selected` : summary}
        </span>
        <button
          type="button"
          className="dir-ui-linkish"
          onClick={() => setOpen(false)}
        >
          ✕ Collapse
        </button>
      </div>
      {missing.length > 0 ? (
        <div className="dir-ui-agent-skill-missing" role="status">
          {missing.map((id) => (
            <span key={id} className="dir-ui-agent-missing-row">
              <span className="dir-ui-agent-missing-badge">missing</span>
              <span className="mono">{id}</span>
              {!readOnly ? (
                <button
                  type="button"
                  className="dir-ui-linkish quiet"
                  onClick={() => toggle(id)}
                >
                  remove
                </button>
              ) : null}
            </span>
          ))}
        </div>
      ) : null}
      {error ? (
        <div className="dir-ui-agent-skill-empty">
          The {noun} catalog could not be loaded; edit the list in the source
          below instead.
        </div>
      ) : items === null ? (
        <div className="dir-ui-agent-skill-empty">Loading {noun}s…</div>
      ) : (
        <>
          <Input
            value={filter}
            onChange={setFilter}
            placeholder={`Search ${noun}s…`}
            aria-label={`search ${noun}s`}
            spellCheck={false}
            className="dir-ui-edit-input"
          />
          <ul className="dir-ui-agent-skill-list tall">
            {visible.length === 0 ? (
              <li className="dir-ui-agent-skill-empty">
                {needle ? `No ${noun}s match.` : `No ${noun}s available.`}
              </li>
            ) : (
              visible.map((s) => (
                <li key={s.id}>
                  <label className="dir-ui-checkrow dir-ui-af-pick-row">
                    <input
                      type="checkbox"
                      checked={isChecked(s.id)}
                      disabled={readOnly}
                      onChange={() => toggle(s.id)}
                    />
                    <span className="dir-ui-af-pick-body">
                      <span className="dir-ui-af-pick-name">
                        {s.iconToken ? (
                          <span className="dir-ui-nav-ico">
                            <TokenIcon token={s.iconToken} size={14} />
                          </span>
                        ) : null}
                        {s.label}
                        {s.sublabel ? (
                          <span className="dir-ui-agent-skill-id mono">
                            {s.sublabel}
                          </span>
                        ) : null}
                      </span>
                      {s.desc ? (
                        <span className="dir-ui-af-pick-desc">{s.desc}</span>
                      ) : null}
                    </span>
                  </label>
                </li>
              ))
            )}
          </ul>
        </>
      )}
    </div>
  )
}

// ───────────────────────────── the form ──────────────────────────────

export function AgentForm(ctx: FormContext) {
  const {
    host,
    draft,
    editDraft,
    readOnly,
    fieldId,
    entryKey,
    nameValue,
    descriptionValue,
    setName,
    setDescription,
    creating,
  } = ctx

  const skills = readFrontmatterStringList(draft, 'skills').values
  const delegates = readFrontmatterStringList(draft, 'delegates_to')
  const leaf = /^true$/i.test(readFrontmatterField(draft, ['leaf']).value)
  const model = readFrontmatterField(draft, ['model']).value.trim()
  const icon = readFrontmatterField(draft, ['icon']).value.trim()

  const skillCatalog = useCatalog(host, fetchSkills)
  const agentCatalog = useCatalog(host, fetchAgents)
  const modelCatalog = useCatalog(host, fetchModels)
  const delegateItems =
    agentCatalog.items === null
      ? null
      : agentCatalog.items.filter((a) => a.id !== entryKey)
  const modelKnown =
    modelCatalog.items === null ||
    model === '' ||
    modelCatalog.items.some((m) => m.id === model)

  const toggleIn = (key: 'skills' | 'delegates_to', current: string[]) => {
    return (id: string) => {
      const next = current.includes(id)
        ? current.filter((s) => s !== id)
        : [...current, id]
      editDraft(setFrontmatterStringList(draft, key, next))
    }
  }
  const setLeaf = (next: boolean) => {
    editDraft(
      next
        ? setFrontmatterField(draft, 'leaf', 'true', true)
        : withoutFrontmatterFields(draft, ['leaf']),
    )
  }
  const setModel = (next: string) => {
    editDraft(
      next
        ? setFrontmatterField(draft, 'model', next)
        : withoutFrontmatterFields(draft, ['model']),
    )
  }
  /** The avatar IS the tree icon: one pick writes `icon` (the harness
   * token the session tree renders) and the matching emoji `logo` (what
   * the agent list rows show) in lockstep; clearing removes both. One
   * edit carrying both fields — two editDraft calls in a row would lose
   * the first (both close over the same `draft`). */
  const setAvatar = (preset: { emoji: string; token: string } | null) => {
    if (preset === null) {
      editDraft(withoutFrontmatterFields(draft, ['logo', 'icon']))
      return
    }
    const withLogo = setFrontmatterField(draft, 'logo', preset.emoji)
    editDraft(setFrontmatterField(withLogo, 'icon', preset.token, true))
  }

  const knownIn = (items: PickItem[] | null) => {
    const ids = new Set((items ?? []).map((s) => s.id))
    return (id: string) => ids.has(id) || ids.has(`${id}/index`)
  }
  const missingSkills =
    skillCatalog.items === null
      ? []
      : skills.filter((id) => !knownIn(skillCatalog.items)(id))
  const missingDelegates =
    delegateItems === null
      ? []
      : delegates.values.filter((id) => !knownIn(delegateItems)(id))

  const derived = useMemo(() => slugify(nameValue), [nameValue])

  return (
    <div className="dir-ui-af">
      <Section
        title="Identity"
        hint="Give the agent a recognizable name and a concise purpose."
      >
        <Row
          label="Avatar"
          hint={
            icon
              ? `${icon} — shown on the session in the side panel`
              : 'pick the icon this agent shows in the session tree'
          }
        >
          <div
            className="dir-ui-af-icon-grid"
            role="radiogroup"
            aria-label="agent avatar"
          >
            {LOGO_PRESETS.map((p) => (
              <button
                key={p.token}
                type="button"
                role="radio"
                aria-checked={icon === p.token}
                title={p.token}
                aria-label={`avatar ${p.token}`}
                disabled={readOnly}
                className={`dir-ui-af-icon-tile${icon === p.token ? ' active' : ''}`}
                onClick={() => setAvatar(icon === p.token ? null : p)}
              >
                <TokenIcon token={p.token} />
              </button>
            ))}
          </div>
        </Row>
        <Row
          label="Name"
          hint={
            creating
              ? derived
                ? `→ ${derived}.md`
                : 'the file name derives from it'
              : undefined
          }
        >
          <Input
            id={`${fieldId}-name`}
            value={nameValue}
            onChange={setName}
            placeholder="e.g. Deep Research Agent"
            aria-label="agent name"
            preserveCase
            required
            spellCheck={false}
            readOnly={readOnly}
            className="dir-ui-edit-input"
          />
        </Row>
        <Row label="Description">
          <textarea
            id={`${fieldId}-description`}
            value={descriptionValue}
            onChange={(event) => setDescription(event.currentTarget.value)}
            placeholder="What does this agent do?"
            readOnly={readOnly}
            rows={2}
            className="dir-ui-edit-textarea"
          />
        </Row>
      </Section>

      <Section
        title="Behavior & capabilities"
        hint="The markdown below this form is the system prompt; attach the skills it can rely on."
      >
        <Row label="Skills">
          <CollapsiblePicker
            noun="skill"
            emptyLabel="Add skills — none selected means every skill"
            items={skillCatalog.items}
            error={skillCatalog.error}
            selected={skills}
            missing={missingSkills}
            isChecked={(id) =>
              skills.includes(id) ||
              (id.endsWith('/index') &&
                skills.includes(id.slice(0, -'/index'.length)))
            }
            toggle={toggleIn('skills', skills)}
            readOnly={readOnly}
          />
        </Row>
      </Section>

      <Section
        title="Execution"
        hint="Optionally pin the default model sessions run as this agent."
      >
        <Row
          label="Model"
          hint={modelKnown ? undefined : 'not in the model catalog'}
        >
          <select
            id={`${fieldId}-model`}
            value={model}
            disabled={readOnly}
            onChange={(event) => setModel(event.currentTarget.value)}
            className="dir-ui-edit-input dir-ui-agent-model"
          >
            <option value="">Default — the send decides</option>
            {!modelKnown ? (
              <option value={model}>{model} (unavailable)</option>
            ) : null}
            {(modelCatalog.items ?? []).map((m) => (
              <option key={m.id} value={m.id}>
                {m.id}
              </option>
            ))}
          </select>
        </Row>
      </Section>

      <Section
        title="Delegation"
        hint="Which agents this one may hand work to when orchestrating."
      >
        <Row label="Leaf">
          <label className="dir-ui-checkrow dir-ui-agent-leaf">
            <input
              type="checkbox"
              checked={leaf}
              disabled={readOnly}
              onChange={(event) => setLeaf(event.currentTarget.checked)}
            />
            <span>May not delegate (leaf agent)</span>
          </label>
        </Row>
        {leaf ? null : (
          <Row label="Delegates">
            <CollapsiblePicker
              noun="agent"
              emptyLabel="Add agents — none selected means every agent"
              items={delegateItems}
              error={agentCatalog.error}
              selected={delegates.values}
              missing={missingDelegates}
              isChecked={(id) => delegates.values.includes(id)}
              toggle={toggleIn('delegates_to', delegates.values)}
              readOnly={readOnly}
            />
          </Row>
        )}
      </Section>
    </div>
  )
}
