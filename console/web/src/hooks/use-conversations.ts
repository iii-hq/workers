/**
 * Server-backed conversation store over the session-manager worker.
 *
 * Sessions ARE the conversations: the sidebar lists `session::list`, the
 * transcript hydrates from `session::messages`, and live rendering
 * reconciles the six `session::*` trigger types (per
 * `session-manager/architecture/integration.md`). localStorage no longer
 * persists transcripts — only UI affordances (active id, last model).
 *
 * Lifecycle:
 * - "new chat" is a LOCAL DRAFT (`draft: true`, id `console-<uuid>`); the
 *   session is materialised by `ensureSession` on the first send so empty
 *   chats never litter the store.
 * - rename / model / thinking / mode / skills changes write through
 *   `session::set-meta`. The console owns the metadata convention
 *   `{ surface, model, thinking_level, mode, skills, title_manual }`;
 *   metadata replaces WHOLESALE, so the full object is always sent.
 * - delete writes through `session::delete`; the sidebar prunes on the
 *   `session::deleted` event (and optimistically).
 * - transcript content reconciles `message-added` / `message-updated`
 *   snapshots by entry, keeping the highest revision per entry
 *   (at-least-once, unordered delivery).
 *
 * When `serverEnabled` is false (mock backends / Storybook) everything is
 * in-memory drafts with the same API surface.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  DEFAULT_SYSTEM_PROMPT_STATE,
  type SystemPromptAddon,
  type SystemPromptState,
} from '@/components/chat/system-prompt-selection'
import { requestComposerFocus } from '@/lib/composer-insert'
import { getIiiClient, type IIIConnectionState } from '@/lib/iii-client'
import { newSessionId } from '@/lib/session-id'
import {
  deleteSession,
  ensureSession as ensureSessionApi,
  fetchTranscript,
  getSession,
  listSessions,
  setSessionDraft,
  setSessionMeta,
} from '@/lib/sessions/api'
import {
  applyEntryUpsert,
  clearTransientFlags,
  transcriptToMessages,
} from '@/lib/sessions/entry-mapper'
import {
  subscribeSessionDirectory,
  subscribeSessionTranscript,
} from '@/lib/sessions/events'
import type {
  MetaUpdatedEvent,
  SessionMeta,
  StatusChangedEvent,
  TranscriptItem,
} from '@/lib/sessions/types'
import {
  loadActiveId,
  loadLastModel,
  loadLastThinkingLevel,
  saveActiveId,
  saveLastModel,
  saveLastThinkingLevel,
  saveRecentProject,
} from '@/lib/storage'
import { releaseConsoleClaimIfAny } from '@/lib/worktree-claims'
import {
  type Conversation,
  type ConversationMetadataEdits,
  DEFAULT_MODE,
  DEFAULT_THINKING_LEVEL,
  type Message,
  type MessagePatch,
  type Mode,
  type ModelId,
  type SubagentAppearance,
  type SubagentColor,
  type SubagentIcon,
  type ThinkingLevel,
} from '@/types/chat'

function uid(): string {
  return Math.random().toString(36).slice(2) + Date.now().toString(36)
}

/** Parse an id list serialized locally with JSON.stringify. */
function sessionIdsFromSignature(signature: string): string[] {
  return JSON.parse(signature) as string[]
}

/** Composer-draft save cadence (`session::set-draft` is event-silent, so the
 *  only costs are the RPC and one JSONL append per flush). */
const DRAFT_SAVE_DEBOUNCE_MS = 500

function deriveTitle(text: string): string {
  const clean = text.replace(/\s+/g, ' ').trim().toLowerCase()
  if (!clean) return 'new chat'
  return clean.length > 32 ? `${clean.slice(0, 32)}…` : clean
}

function emptyConversation(
  defaultModel: ModelId | null,
  defaultThinkingLevel: ThinkingLevel,
): Conversation {
  const now = Date.now()
  return {
    id: newSessionId(),
    title: 'new chat',
    model: defaultModel,
    thinkingLevel: defaultThinkingLevel,
    mode: DEFAULT_MODE,
    // Drafts start with no working dir; ChatView pre-fills the stack's
    // default folder (harness::filesystem::info, validated against the live
    // shell) once known — always visible in the picker chip. What stays
    // forbidden is silently inheriting the LAST-USED dir: that is what once
    // made a chat operate in the wrong directory without the user choosing it.
    workingDir: null,
    messages: [],
    started: false,
    status: 'idle',
    draft: true,
    hydrated: true,
    createdAt: now,
    updatedAt: now,
  }
}

/** A chat nobody has written in yet: still local, no transcript, no draft
    text. Two of these are the same chat as far as anyone can tell. */
export function isUntouchedDraft(conversation: Conversation): boolean {
  return (
    conversation.draft === true &&
    conversation.messages.length === 0 &&
    (conversation.draftText ?? '') === ''
  )
}

function isMode(v: unknown): v is Mode {
  return v === 'ask' || v === 'agent'
}

const SUBAGENT_ICON_VALUES = new Set<SubagentIcon>([
  'agent',
  'code',
  'search',
  'terminal',
  'database',
  'test',
  'review',
  'docs',
  'design',
])
const SUBAGENT_COLOR_VALUES = new Set<SubagentColor>([
  'neutral',
  'blue',
  'purple',
  'teal',
  'green',
  'amber',
  'rose',
])

function decodeSubagentAppearance(
  value: unknown,
): SubagentAppearance | undefined {
  if (typeof value !== 'object' || value === null) return undefined
  const raw = value as Record<string, unknown>
  const name = typeof raw.name === 'string' ? raw.name.trim() : ''
  if (!name) return undefined
  const icon =
    typeof raw.icon === 'string' &&
    SUBAGENT_ICON_VALUES.has(raw.icon as SubagentIcon)
      ? (raw.icon as SubagentIcon)
      : undefined
  const color =
    typeof raw.color === 'string' &&
    SUBAGENT_COLOR_VALUES.has(raw.color as SubagentColor)
      ? (raw.color as SubagentColor)
      : undefined
  return {
    name: Array.from(name).slice(0, 48).join(''),
    ...(icon ? { icon } : {}),
    ...(color ? { color } : {}),
  }
}

/**
 * Wire shape for `metadata.system_prompt`. Undefined for the `default`
 * choice, so the key is absent rather than present-and-empty (same
 * convention as `fs_scope` / `memory_bank`).
 */
function encodeSystemPrompt(
  s: SystemPromptState | undefined,
): Record<string, unknown> | undefined {
  if (!s) return undefined
  const addons = (s.addons ?? []).map(({ kind, name, body }) => ({
    kind,
    name,
    body,
  }))
  if (s.choice === 'default' && addons.length === 0) return undefined
  return {
    choice:
      s.choice === 'default'
        ? 'default'
        : s.choice === 'custom'
          ? 'custom'
          : { named: s.choice.named },
    strategy: s.strategy,
    named_body: s.namedBody,
    ...(addons.length > 0 ? { addons } : {}),
  }
}

/**
 * Decode `metadata.system_prompt` defensively — it is untrusted wire JSON,
 * and an unreadable value must degrade to the default rather than throw.
 *
 * ponytail: only the `{named}` and `default`-with-addons shapes round-trip.
 * `custom` cannot reach here today because the sole writer is the
 * new-session picker, which renders no `custom…` row; extend this if
 * another surface ever persists a free-text choice.
 */
function decodeSystemPrompt(v: unknown): SystemPromptState {
  if (typeof v !== 'object' || v === null) return DEFAULT_SYSTEM_PROMPT_STATE
  const md = v as Record<string, unknown>
  const choice = md.choice
  const named =
    typeof choice === 'object' &&
    choice !== null &&
    typeof (choice as Record<string, unknown>).named === 'string'
      ? ((choice as Record<string, unknown>).named as string)
      : null
  const addons: SystemPromptAddon[] = Array.isArray(md.addons)
    ? (md.addons as unknown[]).flatMap((a): SystemPromptAddon[] => {
        if (typeof a !== 'object' || a === null) return []
        const r = a as Record<string, unknown>
        return (r.kind === 'prompt' || r.kind === 'skill') &&
          typeof r.name === 'string' &&
          typeof r.body === 'string'
          ? [{ kind: r.kind, name: r.name, body: r.body }]
          : []
      })
    : []
  if (!named && addons.length === 0) return DEFAULT_SYSTEM_PROMPT_STATE
  return {
    choice: named ? { named } : 'default',
    strategy: md.strategy === 'override' ? 'override' : 'enrich',
    namedBody: named && typeof md.named_body === 'string' ? md.named_body : '',
    customText: '',
    addons,
  }
}

function decodeSkills(v: unknown): string[] | undefined {
  if (!Array.isArray(v)) return undefined
  const skills = [
    ...new Set(v.filter((id): id is string => typeof id === 'string' && !!id)),
  ]
  return skills.length > 0 ? skills : undefined
}

function decodeSessionSelections(
  md: Record<string, unknown>,
  started: boolean,
): Pick<Conversation, 'systemPrompt' | 'skills' | 'legacySkillMigration'> {
  const systemPrompt = decodeSystemPrompt(md.system_prompt)
  const skills = decodeSkills(md.skills)
  const hasLegacySkills = systemPrompt.addons.some(
    (addon) => addon.kind === 'skill',
  )
  return {
    systemPrompt,
    skills,
    legacySkillMigration:
      !started && hasLegacySkills
        ? { state: 'candidate', metadata: md }
        : undefined,
  }
}

function finalizeLegacySkillMigration(c: Conversation): Conversation {
  const migration = c.legacySkillMigration
  if (migration?.state !== 'candidate') return c

  const legacySkills = decodeSystemPrompt(migration.metadata.system_prompt)
    .addons.filter((addon) => addon.kind === 'skill')
    .map((addon) => addon.name)
  const skills = Object.hasOwn(migration.edits ?? {}, 'skills')
    ? migration.edits?.skills
    : Array.isArray(migration.metadata.skills)
      ? decodeSkills(migration.metadata.skills)
      : legacySkills.length
        ? [...new Set(legacySkills)]
        : undefined
  const systemPrompt = c.systemPrompt
    ? {
        ...c.systemPrompt,
        addons: c.systemPrompt.addons.filter((addon) => addon.kind !== 'skill'),
      }
    : undefined
  const migrated = { ...c, systemPrompt, skills }
  const metadata = migrationMetadataFor(migrated, migration.metadata)

  return {
    ...migrated,
    sessionMetadata: metadata,
    legacySkillMigration: {
      state: 'ready',
      metadata,
      ...(migration.edits ? { edits: migration.edits } : {}),
    },
  }
}

function reconcileLegacySkillMigration(
  previous: Conversation | undefined,
  next: Conversation,
): Conversation {
  const migration = previous?.legacySkillMigration
  if (!migration) return next
  const edits = migration.edits
  const current = edits ? { ...next, ...edits } : next

  if (migration.state === 'candidate') {
    if (!edits) return next
    const incomingCandidate =
      next.legacySkillMigration?.state === 'candidate'
        ? next.legacySkillMigration
        : undefined
    const candidate = incomingCandidate
      ? {
          ...incomingCandidate,
          metadata: {
            ...migration.metadata,
            ...incomingCandidate.metadata,
          },
          edits,
        }
      : migration
    return {
      ...current,
      ...(!incomingCandidate && !Object.hasOwn(edits, 'systemPrompt')
        ? { systemPrompt: previous.systemPrompt }
        : {}),
      legacySkillMigration: candidate,
    }
  }

  if (next.legacySkillMigration?.state === 'candidate') {
    return finalizeLegacySkillMigration({
      ...current,
      legacySkillMigration: {
        ...next.legacySkillMigration,
        metadata: migration.metadata
          ? {
              ...migration.metadata,
              ...next.legacySkillMigration.metadata,
            }
          : next.legacySkillMigration.metadata,
        ...(edits ? { edits } : {}),
      },
    })
  }
  if (migration.state === 'ready' && edits) {
    return {
      ...current,
      legacySkillMigration: { ...migration, edits },
    }
  }
  return {
    ...current,
    legacySkillMigration: {
      state: 'empty',
      ...(migration.metadata ? { metadata: migration.metadata } : {}),
      ...(edits ? { edits } : {}),
    },
  }
}

/** The console's session metadata convention (replaces wholesale on writes). */
export function metadataFor(
  c: Pick<
    Conversation,
    | 'model'
    | 'thinkingLevel'
    | 'mode'
    | 'titleManual'
    | 'workingDir'
    | 'memoryBank'
    | 'systemPrompt'
    | 'skills'
    | 'sessionMetadata'
  >,
): Record<string, unknown> {
  const systemPrompt = encodeSystemPrompt(c.systemPrompt)
  // session::set-meta replaces metadata wholesale. Keep keys owned by the
  // harness (parent linkage and sub-agent presentation) and other surfaces,
  // while rebuilding the console-owned keys below so clearing one really
  // removes it.
  const {
    surface: _surface,
    model: _model,
    thinking_level: _thinkingLevel,
    mode: _mode,
    title_manual: _titleManual,
    fs_scope: _fsScope,
    memory_bank: _memoryBank,
    system_prompt: _systemPrompt,
    skills: _skills,
    ...preserved
  } = c.sessionMetadata ?? {}
  return {
    ...preserved,
    surface: 'console',
    ...(c.model ? { model: c.model } : {}),
    ...(c.thinkingLevel && c.thinkingLevel !== DEFAULT_THINKING_LEVEL
      ? { thinking_level: c.thinkingLevel }
      : {}),
    mode: c.mode,
    ...(c.titleManual ? { title_manual: true } : {}),
    ...(c.workingDir ? { fs_scope: { root: c.workingDir } } : {}),
    ...(c.memoryBank ? { memory_bank: c.memoryBank } : {}),
    ...(systemPrompt ? { system_prompt: systemPrompt } : {}),
    ...(c.skills?.length ? { skills: c.skills } : {}),
  }
}

const CONSOLE_METADATA_KEYS = [
  'surface',
  'model',
  'thinking_level',
  'mode',
  'title_manual',
  'fs_scope',
  'memory_bank',
  'system_prompt',
  'skills',
] as const

function migrationMetadataFor(
  c: Conversation,
  base: Record<string, unknown>,
): Record<string, unknown> {
  const metadata = { ...base }
  for (const key of CONSOLE_METADATA_KEYS) delete metadata[key]
  return { ...metadata, ...metadataFor(c) }
}

export function applyConversationMetadataPatch(
  c: Conversation,
  patch: ConversationMetadataEdits,
  now = Date.now(),
): Conversation {
  const normalized: ConversationMetadataEdits = Object.hasOwn(patch, 'skills')
    ? { ...patch, skills: patch.skills?.length ? patch.skills : undefined }
    : patch
  const migration = c.legacySkillMigration
  return {
    ...c,
    ...normalized,
    legacySkillMigration: migration
      ? {
          ...migration,
          edits: { ...migration.edits, ...normalized },
        }
      : undefined,
    updatedAt: now,
  }
}

export function metadataForWrite(c: Conversation): Record<string, unknown> {
  const migration = c.legacySkillMigration
  return migration?.metadata
    ? migrationMetadataFor(c, migration.metadata)
    : metadataFor(c)
}

export function preSendMetaUpdate(c: Conversation): {
  session_id: string
  metadata: Record<string, unknown>
} | null {
  const migration = c.legacySkillMigration
  return !c.draft && c.started !== true && migration?.state === 'ready'
    ? {
        session_id: c.id,
        metadata: migrationMetadataFor(c, migration.metadata),
      }
    : null
}

export function completePreSendMetaUpdate(
  c: Conversation,
  pendingEdits: ConversationMetadataEdits | undefined,
): Conversation {
  const migration = c.legacySkillMigration
  if (migration?.state !== 'ready' || migration.edits !== pendingEdits) return c
  return {
    ...c,
    legacySkillMigration: {
      state: 'empty',
      metadata: migration.metadata,
      ...(migration.edits ? { edits: migration.edits } : {}),
    },
  }
}

function conversationFromMeta(
  meta: SessionMeta,
  started = meta.message_count > 0,
  migrationPending = false,
): Conversation {
  const md = meta.metadata ?? {}
  const { systemPrompt, skills, legacySkillMigration } =
    decodeSessionSelections(md, started && !migrationPending)
  return {
    id: meta.session_id,
    title: meta.title || meta.session_id,
    titleManual: md.title_manual === true,
    model:
      typeof md.model === 'string' && md.model.length > 0 ? md.model : null,
    thinkingLevel:
      typeof md.thinking_level === 'string' && md.thinking_level.length > 0
        ? md.thinking_level
        : DEFAULT_THINKING_LEVEL,
    mode: isMode(md.mode) ? md.mode : DEFAULT_MODE,
    workingDir:
      typeof md.fs_scope === 'object' &&
      md.fs_scope !== null &&
      typeof (md.fs_scope as Record<string, unknown>).root === 'string' &&
      ((md.fs_scope as Record<string, unknown>).root as string).length > 0
        ? ((md.fs_scope as Record<string, unknown>).root as string)
        : null,
    memoryBank:
      typeof md.memory_bank === 'string' && md.memory_bank.length > 0
        ? md.memory_bank
        : null,
    systemPrompt,
    skills,
    legacySkillMigration,
    started,
    sessionMetadata: md,
    serverMetaUpdatedAt: meta.updated_at,
    serverMetadataUpdatedAt: meta.updated_at,
    serverStatusUpdatedAt: meta.updated_at,
    subagentAppearance: decodeSubagentAppearance(md.subagent_display),
    parentId:
      typeof md.parent_session_id === 'string'
        ? md.parent_session_id
        : undefined,
    parentFunctionCallId:
      typeof md.function_call_id === 'string' ? md.function_call_id : undefined,
    depth: typeof md.depth === 'number' ? md.depth : undefined,
    spawnedBy:
      md.spawned_by === 'trigger' || md.spawned_by === 'agent'
        ? md.spawned_by
        : undefined,
    draftText:
      typeof meta.draft === 'string' && meta.draft.length > 0
        ? meta.draft
        : undefined,
    messages: [],
    status: meta.status,
    statusReason: meta.status_reason,
    hydrated: false,
    createdAt: meta.created_at,
    updatedAt: meta.updated_at,
  }
}

/** Apply one partial metadata event without letting unordered delivery roll
 * back a newer title or sub-agent presentation event. */
export function applyConversationMetadataEvent(
  conversation: Conversation,
  event: MetaUpdatedEvent,
): Conversation {
  if (
    conversation.serverMetadataUpdatedAt !== undefined &&
    event.timestamp < conversation.serverMetadataUpdatedAt
  ) {
    return conversation
  }
  const md = event.metadata ?? {}
  const { systemPrompt, skills, legacySkillMigration } =
    decodeSessionSelections(
      md,
      conversation.started === true && !conversation.legacySkillMigration,
    )
  const next: Conversation = {
    ...conversation,
    title: event.title || conversation.title,
    titleManual: md.title_manual === true || conversation.titleManual,
    model:
      typeof md.model === 'string' && md.model.length > 0
        ? md.model
        : conversation.model,
    thinkingLevel:
      typeof md.thinking_level === 'string' && md.thinking_level.length > 0
        ? md.thinking_level
        : DEFAULT_THINKING_LEVEL,
    mode: isMode(md.mode) ? md.mode : conversation.mode,
    memoryBank:
      typeof md.memory_bank === 'string' && md.memory_bank.length > 0
        ? md.memory_bank
        : null,
    systemPrompt,
    skills,
    legacySkillMigration,
    sessionMetadata: md,
    subagentAppearance: decodeSubagentAppearance(md.subagent_display),
    serverMetaUpdatedAt: Math.max(
      conversation.serverMetaUpdatedAt ?? -Infinity,
      event.timestamp,
    ),
    serverMetadataUpdatedAt: event.timestamp,
    parentId:
      typeof md.parent_session_id === 'string'
        ? md.parent_session_id
        : conversation.parentId,
    parentFunctionCallId:
      typeof md.function_call_id === 'string'
        ? md.function_call_id
        : conversation.parentFunctionCallId,
    depth: typeof md.depth === 'number' ? md.depth : conversation.depth,
    spawnedBy:
      md.spawned_by === 'trigger' || md.spawned_by === 'agent'
        ? md.spawned_by
        : conversation.spawnedBy,
    updatedAt: Math.max(conversation.updatedAt, event.timestamp),
  }
  return reconcileLegacySkillMigration(conversation, next)
}

/** Apply one partial lifecycle event without letting an older delivery turn a
 * terminal sub-agent back into an active chip. */
export function applyConversationStatusEvent(
  conversation: Conversation,
  event: StatusChangedEvent,
): Conversation {
  if (
    conversation.serverStatusUpdatedAt !== undefined &&
    event.timestamp < conversation.serverStatusUpdatedAt
  ) {
    return conversation
  }
  return {
    ...conversation,
    status: event.status,
    statusReason: event.status_reason,
    serverMetaUpdatedAt: Math.max(
      conversation.serverMetaUpdatedAt ?? -Infinity,
      event.timestamp,
    ),
    serverStatusUpdatedAt: event.timestamp,
    // Turn over: drop dangling streaming/running flags.
    messages:
      event.status === 'working'
        ? conversation.messages
        : clearTransientFlags(conversation.messages),
    updatedAt: Math.max(conversation.updatedAt, event.timestamp),
  }
}

export function applyCatalogModelFallback(
  conversations: Conversation[],
  validModels: ReadonlySet<string>,
  fallbackModel: ModelId,
): Conversation[] {
  let changed = false
  const next = conversations.map((c) => {
    if (c.model && validModels.has(c.model)) return c
    // A discovered session (sub-agent, other-surface) with no model choice
    // must stay null — inventing one here would misreport the model the
    // session actually runs; the chat view derives it from the transcript.
    // Only console drafts get the catalog default.
    if (!c.model && !c.draft) return c
    changed = true
    return { ...c, model: fallbackModel }
  })
  return changed ? next : conversations
}

/** Keep a just-selected session even if `session::created` has not yet
 *  inserted it into the sidebar list. Without this, the boot-time "always
 *  have an active chat" effect snaps back to conversations[0]. */
export function resolveActiveConversationId(input: {
  conversationIds: readonly string[]
  activeId: string | null
  pendingSelectId: string | null
}): { activeId: string | null; pendingSelectId: string | null } {
  const { conversationIds, activeId, pendingSelectId } = input
  if (conversationIds.length === 0) {
    return { activeId, pendingSelectId }
  }
  if (pendingSelectId) {
    if (conversationIds.includes(pendingSelectId)) {
      return { activeId: pendingSelectId, pendingSelectId: null }
    }
    return { activeId: pendingSelectId, pendingSelectId }
  }
  if (!activeId || !conversationIds.includes(activeId)) {
    return { activeId: conversationIds[0], pendingSelectId: null }
  }
  return { activeId, pendingSelectId: null }
}

/**
 * Mark every backgrounded server-backed conversation stale so the next
 * activation re-hydrates it. A transcript subscription exists only for the
 * ACTIVE session, so entry events emitted while a session is backgrounded
 * are lost — a function trigger caught mid-snapshot freezes as `ƒ …` with an
 * empty request/response until durable truth is re-fetched. Returns the
 * same array when nothing changed.
 */
export function markBackgroundedStale(
  conversations: Conversation[],
  activeId: string | null,
): Conversation[] {
  let changed = false
  const next = conversations.map((c) => {
    if (c.id === activeId || c.draft || !c.hydrated) return c
    changed = true
    return { ...c, hydrated: false }
  })
  return changed ? next : conversations
}

/** Mark server sessions that have no mounted chat panel stale. Unlike the
 * legacy active-only helper, this keeps every visible side-by-side panel
 * hydrated and live. */
export function markUnwatchedStale(
  conversations: Conversation[],
  watchedIds: ReadonlySet<string>,
): Conversation[] {
  let changed = false
  const next = conversations.map((c) => {
    if (watchedIds.has(c.id) || c.draft || !c.hydrated) return c
    changed = true
    return { ...c, hydrated: false }
  })
  return changed ? next : conversations
}

export function mergeConversationMeta(
  existing: Conversation | undefined,
  meta: SessionMeta,
): Conversation {
  const started = existing?.started === true || meta.message_count > 0
  const mapped = conversationFromMeta(
    meta,
    started,
    existing?.legacySkillMigration !== undefined,
  )
  if (!existing || existing.draft) return mapped
  const metadataIsStale =
    existing.serverMetadataUpdatedAt !== undefined &&
    meta.updated_at < existing.serverMetadataUpdatedAt
  const statusIsStale =
    existing.serverStatusUpdatedAt !== undefined &&
    meta.updated_at < existing.serverStatusUpdatedAt
  const merged: Conversation = {
    ...mapped,
    messages: existing.messages,
    hydrated: existing.hydrated,
    serverMetaUpdatedAt: Math.max(
      existing.serverMetaUpdatedAt ?? -Infinity,
      meta.updated_at,
    ),
    updatedAt: Math.max(existing.updatedAt, meta.updated_at),
  }
  if (metadataIsStale) {
    Object.assign(merged, {
      title: existing.title,
      titleManual: existing.titleManual,
      model: existing.model,
      thinkingLevel: existing.thinkingLevel,
      mode: existing.mode,
      workingDir: existing.workingDir,
      memoryBank: existing.memoryBank,
      systemPrompt: existing.systemPrompt,
      skills: existing.skills,
      legacySkillMigration: existing.legacySkillMigration,
      sessionMetadata: existing.sessionMetadata,
      subagentAppearance: existing.subagentAppearance,
      parentId: existing.parentId,
      parentFunctionCallId: existing.parentFunctionCallId,
      depth: existing.depth,
      spawnedBy: existing.spawnedBy,
      serverMetadataUpdatedAt: existing.serverMetadataUpdatedAt,
    })
  }
  if (statusIsStale) {
    merged.status = existing.status
    merged.statusReason = existing.statusReason
    merged.serverStatusUpdatedAt = existing.serverStatusUpdatedAt
  }
  return reconcileLegacySkillMigration(existing, merged)
}

/** Merge the boot-time session list without dropping sessions discovered by
 * a concurrent `session::created` event. The list RPC may have been captured
 * before that event, so treating it as a replacement can invalidate the
 * active conversation and send the user back to the local draft. */
export function mergeSessionListSnapshot(
  previous: Conversation[],
  metas: SessionMeta[],
): Conversation[] {
  const drafts = previous.filter((conversation) => conversation.draft)
  const byId = new Map(
    previous.map((conversation) => [conversation.id, conversation]),
  )
  const listed = metas.map((meta) =>
    mergeConversationMeta(byId.get(meta.session_id), meta),
  )
  const listedIds = new Set(listed.map((conversation) => conversation.id))
  const concurrent = previous.filter(
    (conversation) => !conversation.draft && !listedIds.has(conversation.id),
  )
  return [
    ...drafts.filter((conversation) => !listedIds.has(conversation.id)),
    ...concurrent,
    ...listed,
  ]
}

/** A fresh reconnect directory row wins unless this same refresh already has
 * a definitive exact not-found/deleted tombstone for the session. */
export function shouldAcceptReconnectDirectoryRow(input: {
  currentGeneration: number
  missingGeneration?: number
}): boolean {
  return input.missingGeneration !== input.currentGeneration
}

export function missingGenerationForDirectoryRefresh(
  tombstone:
    | { lookupGeneration: number; directoryRefreshGeneration: number }
    | undefined,
  refreshGeneration: number,
): number | undefined {
  return tombstone?.directoryRefreshGeneration === refreshGeneration
    ? tombstone.lookupGeneration
    : undefined
}

/** Increment only one panel session's lifecycle token. */
export function bumpSessionWatchEpoch(
  epochs: Map<string, number>,
  sessionId: string,
): number {
  const next = (epochs.get(sessionId) ?? 0) + 1
  epochs.set(sessionId, next)
  return next
}

export function appendMessageToConversation(
  c: Conversation,
  message: Message,
  now = Date.now(),
): Conversation {
  const existingIndex = c.messages.findIndex((item) => item.id === message.id)
  const existing = existingIndex === -1 ? undefined : c.messages[existingIndex]
  const preservesDurableNotice =
    existing?.role === 'system' &&
    message.role === 'system' &&
    message.provisional === true &&
    existing.provisional !== true
  const messages =
    existingIndex === -1
      ? [...c.messages, message]
      : preservesDurableNotice
        ? c.messages
        : c.messages.map((item, index) =>
            index === existingIndex ? message : item,
          )
  const next: Conversation = {
    ...c,
    messages,
    updatedAt: now,
  }
  if (message.role === 'user') {
    next.status = 'working'
    next.statusReason = undefined
  }
  if (
    !c.titleManual &&
    message.role === 'user' &&
    c.messages.every((m) => m.role !== 'user')
  ) {
    next.title = deriveTitle(message.content)
  }
  return next
}

export interface ConversationsApi {
  conversations: Conversation[]
  activeId: string | null
  active: Conversation | null
  /** Current engine connection, used to avoid presenting cached work as live. */
  connectionState: IIIConnectionState
  /** Exact session ids confirmed absent/deleted by session-manager. */
  missingConversationIds: ReadonlySet<string>
  createNew: () => string
  select: (id: string) => void
  /** Keep one session hydrated and subscribed while a chat panel is mounted. */
  watchConversation: (id: string) => () => void
  rename: (id: string, title: string) => void
  remove: (id: string) => void
  setModel: (id: string, model: ModelId) => void
  /** Persist this session's reasoning effort and remember it for new chats. */
  setThinkingLevel: (id: string, level: ThinkingLevel) => void
  /** Point this chat at a named memory bank (null = worker default). */
  setMemoryBank: (id: string, memoryBank: string | null) => void
  /** This chat's system prompt, chosen on the new-session screen. */
  setSystemPrompt: (id: string, systemPrompt: SystemPromptState) => void
  setSkills: (id: string, skills: string[] | undefined) => void
  setMode: (id: string, mode: Mode) => void
  /** Per-session working directory; null clears a scope that is no longer usable. */
  setWorkingDir: (id: string, dir: string | null) => void
  /**
   * Seed a draft's working dir with the stack default: patches state only
   * while the chat is still a draft with no dir (an explicit pick or a
   * materialised session wins), and deliberately skips the recent-projects
   * list and the re-scope transcript notice — it is a default, not a choice.
   */
  prefillWorkingDir: (id: string, dir: string) => void
  appendMessage: (id: string, message: Message) => void
  updateMessage: (id: string, messageId: string, patch: MessagePatch) => void
  compactConversation: (id: string, marker: Message) => void
  /**
   * Materialise a draft conversation in session-manager before the first
   * send (idempotent). `titleHint` seeds the session title from the prompt.
   */
  ensureSession: (id: string, titleHint?: string) => Promise<void>
  /**
   * Record the composer's live text for a conversation. Kept in a ref (no
   * re-render per keystroke) and, for server-backed sessions, persisted via
   * the debounced event-silent `session::set-draft` so a page refresh
   * restores what the user was typing.
   */
  setDraftText: (id: string, text: string) => void
  /**
   * The composer text to seed when (re)opening a conversation: what this tab
   * last recorded via `setDraftText`, else the server-restored
   * `SessionMeta.draft`. `undefined` when there is nothing to restore.
   */
  getDraftText: (id: string) => string | undefined
}

/**
 * @param catalogKeysForValidation When set, non-matching `conversation.model`
 *   values are rewritten to the first key (catalog load / migration);
 *   `catalogReady` gates it so a stale placeholder catalog can't clobber picks.
 * @param serverEnabled Wire the store to session-manager (real backend only).
 */
/** Live entry upsert captured while a hydration fetch was in flight. */
export type HydrationUpsert = { item: TranscriptItem; updated: boolean }

export interface HydrationRun {
  cancelled: boolean
  connectionEpoch: number
  watchEpoch: number
  upserts: HydrationUpsert[]
}

/** Invalidate reads issued on an older connection before starting fresh
 * post-reconnect hydration for the same mounted panels. */
export function cancelHydrationRunsForSessions(
  sessionIds: readonly string[],
  runs: Map<string, HydrationRun>,
  buffers: Map<string, HydrationUpsert[]>,
): void {
  for (const sessionId of sessionIds) {
    const run = runs.get(sessionId)
    if (run) run.cancelled = true
    runs.delete(sessionId)
    buffers.delete(sessionId)
  }
}

/** Fold a hydration read together with what the live feed did meanwhile:
    replay the buffered upserts on top (same entry id → live wins, the
    fetched snapshot predates them), then re-append live-only messages the
    read didn't return. Without the replay, an update landing mid-fetch is
    clobbered by the older snapshot and `hydrated: true` pins it stale. */
export function mergeHydratedTranscript(
  fetched: Message[],
  live: Message[],
  upserts: HydrationUpsert[],
  opts: { sessionId: string; working: boolean },
): Message[] {
  let messages = fetched
  for (const u of upserts) {
    messages = applyEntryUpsert(messages, u.item, {
      sessionId: opts.sessionId,
      streaming: u.updated ? opts.working : undefined,
      working: opts.working,
    })
  }
  for (const m of live) {
    if (!messages.some((existing) => existing.id === m.id)) {
      messages = [...messages, m]
    }
  }
  return messages
}

export function markDurableStarted(
  c: Conversation,
  turnEstablished: boolean,
): Conversation {
  return {
    ...c,
    started: true,
    legacySkillMigration:
      turnEstablished && c.legacySkillMigration?.state === 'candidate'
        ? undefined
        : c.legacySkillMigration,
  }
}

export function mergeHydratedConversation(
  conversation: Conversation,
  items: TranscriptItem[],
  upserts: HydrationUpsert[],
): Conversation {
  const working = conversation.status === 'working'
  const started =
    conversation.started === true || items.length > 0 || upserts.length > 0
  const hydrated: Conversation = {
    ...conversation,
    messages: mergeHydratedTranscript(
      transcriptToMessages(items, conversation.id, { working }),
      conversation.messages,
      upserts,
      { sessionId: conversation.id, working },
    ),
    started,
    hydrated: true,
  }
  const turnEstablished =
    items.some((item) => item.message?.role === 'assistant') ||
    upserts.some(({ item }) => item.message?.role === 'assistant') ||
    hydrated.messages.some((message) => message.role === 'assistant')
  if (started) return markDurableStarted(hydrated, turnEstablished)
  if (hydrated.legacySkillMigration?.state === 'candidate') {
    return finalizeLegacySkillMigration(hydrated)
  }
  return hydrated.legacySkillMigration?.state === 'ready'
    ? hydrated
    : { ...hydrated, legacySkillMigration: { state: 'empty' } }
}

/** A failed transcript read is still a terminal hydration outcome. Keep the
 * live/optimistic snapshot accumulated in memory so the chat can render it
 * instead of remaining in the initializing state indefinitely. */
export function completeFailedHydration(c: Conversation): Conversation {
  return c.hydrated ? c : { ...c, hydrated: true }
}

export function useConversations(
  catalogKeysForValidation?: readonly string[],
  catalogReady?: boolean,
  serverEnabled?: boolean,
): ConversationsApi {
  const catalogSig =
    catalogKeysForValidation && catalogKeysForValidation.length > 0
      ? [...catalogKeysForValidation].sort().join('\u0001')
      : ''

  const [conversations, setConversations] = useState<Conversation[]>(() => [
    /* Always boot with one local draft so the chat surface has something to
       render. Done in the initializer so StrictMode's double-invoke can't
       create two. */
    emptyConversation(
      loadLastModel(),
      loadLastThinkingLevel() ?? DEFAULT_THINKING_LEVEL,
    ),
  ])
  const conversationsRef = useRef(conversations)
  conversationsRef.current = conversations
  const [activeId, setActiveId] = useState<string | null>(() => loadActiveId())
  const [connectionState, setConnectionState] = useState<IIIConnectionState>(
    serverEnabled ? 'connecting' : 'connected',
  )
  const [hydrationEpoch, setHydrationEpoch] = useState(0)
  const hydrationEpochRef = useRef(0)
  // The revision wakes effects after a same-commit unwatch -> watch, while
  // the per-session epochs ensure mounting panel B never tears down panel A.
  const [watchLifecycleRevision, setWatchLifecycleRevision] = useState(0)
  const watchLifecycleEpochsRef = useRef(new Map<string, number>())
  const [watchedSessionIds, setWatchedSessionIds] = useState<string[]>([])
  const [missingSessionIds, setMissingSessionIds] = useState<string[]>([])
  const watchCountsRef = useRef(new Map<string, number>())
  const visibleSessionIdsRef = useRef<string[]>([])
  const sessionMetaLookupGenerationRef = useRef(new Map<string, number>())
  const missingSessionLookupGenerationRef = useRef(
    new Map<
      string,
      { lookupGeneration: number; directoryRefreshGeneration: number }
    >(),
  )
  const sessionMetaLookupTimersRef = useRef(
    new Map<string, ReturnType<typeof setTimeout>>(),
  )
  const directoryRefreshGenerationRef = useRef(0)
  const pendingSelectIdRef = useRef<string | null>(null)

  /** Highest seen `message-updated` revision per (session, entry). */
  const revisionsRef = useRef(new Map<string, Map<string, number>>())

  /** Upserts received while a hydration fetch is in flight; replayed over
      the fetched snapshot so the older read can't clobber a newer entry. */
  const hydrationBuffersRef = useRef(new Map<string, HydrationUpsert[]>())
  const hydrationRunsRef = useRef(new Map<string, HydrationRun>())
  const hydrationRetryTimersRef = useRef(
    new Map<string, ReturnType<typeof setTimeout>>(),
  )
  const transcriptSubscriptionsRef = useRef(new Map<string, () => void>())
  const transcriptSubscriptionEpochsRef = useRef(new Map<string, number>())

  /* ── Composer drafts (SessionMeta.draft) ──────────────────────────────
     The live editor text lives in refs — one map entry per conversation —
     so keystrokes never re-render the conversation tree. Server persistence
     is debounced through the event-silent `session::set-draft` (see
     `setDraftText` below); reads fall back to the meta-restored
     `conversation.draftText`, so the in-tab value (which knows about sends
     and edits) always wins over the boot snapshot. */
  const draftTextsRef = useRef(new Map<string, string>())
  const lastSavedDraftRef = useRef(new Map<string, string>())
  const pendingDraftRef = useRef<{ id: string; text: string } | null>(null)
  const draftTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  /** Per-session tail of the in-flight `session::set-draft` writes: saves
      chain so an older save can never land after (and clobber) a newer one
      — the case that matters is the post-send CLEAR racing a stale save. */
  const draftSaveChainRef = useRef(new Map<string, Promise<void>>())

  const patchConversation = useCallback(
    (id: string, patch: (c: Conversation) => Conversation) => {
      setConversations((list) => list.map((c) => (c.id === id ? patch(c) : c)))
    },
    [],
  )

  const markConversationMissing = useCallback((id: string) => {
    setMissingSessionIds((current) =>
      current.includes(id) ? current : [...current, id],
    )
  }, [])

  const clearConversationMissing = useCallback((id: string) => {
    setMissingSessionIds((current) =>
      current.includes(id)
        ? current.filter((sessionId) => sessionId !== id)
        : current,
    )
  }, [])

  const invalidateSessionMetaLookup = useCallback((sessionId: string) => {
    const generations = sessionMetaLookupGenerationRef.current
    generations.set(sessionId, (generations.get(sessionId) ?? 0) + 1)
    const timer = sessionMetaLookupTimersRef.current.get(sessionId)
    if (timer) clearTimeout(timer)
    sessionMetaLookupTimersRef.current.delete(sessionId)
  }, [])

  const lookupSessionMeta = useCallback(
    (
      sessionId: string,
      options: { requireWatched?: boolean; retries?: number } = {},
    ) => {
      invalidateSessionMetaLookup(sessionId)
      const generation =
        sessionMetaLookupGenerationRef.current.get(sessionId) ?? 0
      const directoryRefreshGeneration = directoryRefreshGenerationRef.current
      const retries = options.retries ?? 0

      const run = (attempt: number) => {
        sessionMetaLookupTimersRef.current.delete(sessionId)
        void getSession(sessionId)
          .then((meta) => {
            if (
              sessionMetaLookupGenerationRef.current.get(sessionId) !==
              generation
            ) {
              return
            }
            if (
              options.requireWatched &&
              (watchCountsRef.current.get(sessionId) ?? 0) === 0
            ) {
              return
            }
            if (meta) {
              missingSessionLookupGenerationRef.current.delete(sessionId)
              clearConversationMissing(sessionId)
            } else {
              missingSessionLookupGenerationRef.current.set(sessionId, {
                lookupGeneration: generation,
                directoryRefreshGeneration,
              })
              markConversationMissing(sessionId)
            }
            setConversations((current) => {
              const existing = current.find(
                (conversation) => conversation.id === sessionId,
              )
              if (!meta) {
                return existing && !existing.draft
                  ? current.filter(
                      (conversation) => conversation.id !== sessionId,
                    )
                  : current
              }
              if (!existing) return [conversationFromMeta(meta), ...current]
              return current.map((conversation) =>
                conversation.id === sessionId
                  ? mergeConversationMeta(conversation, meta)
                  : conversation,
              )
            })
          })
          .catch(() => {
            const stillCurrent =
              sessionMetaLookupGenerationRef.current.get(sessionId) ===
              generation
            const stillWatched =
              !options.requireWatched ||
              (watchCountsRef.current.get(sessionId) ?? 0) > 0
            if (!stillCurrent || !stillWatched) return
            const retryDelay =
              attempt < retries
                ? 500 * (attempt + 1)
                : options.requireWatched
                  ? 5_000
                  : null
            if (retryDelay === null) return
            // A mounted deep-link panel remains the owner of this retry. Keep
            // trying at a quiet cadence after the short initial backoff; an
            // unwatch or any newer directory event invalidates the generation.
            const timer = setTimeout(() => run(attempt + 1), retryDelay)
            sessionMetaLookupTimersRef.current.set(sessionId, timer)
          })
      }

      run(0)
    },
    [
      clearConversationMissing,
      invalidateSessionMetaLookup,
      markConversationMissing,
    ],
  )

  /* ── Boot + reconnect: refresh durable session metadata ───────────────── */
  useEffect(() => {
    if (!serverEnabled) {
      setConnectionState('connected')
      return
    }
    let cancelled = false
    let off: (() => void) | undefined
    void getIiiClient().then((client) => {
      if (cancelled) return
      off = client.addConnectionStateListener((state) => {
        if (cancelled) return
        setConnectionState(state)
        if (state !== 'connected') return
        // Read the directory before issuing exact lookups. This gives the two
        // RPCs an unambiguous causal order: a later exact null may remove a
        // listed row, while a row created between the calls is seen by the
        // exact read instead of being hidden by a parallel negative result.
        const refreshGeneration = ++directoryRefreshGenerationRef.current
        for (const [
          sessionId,
          tombstone,
        ] of missingSessionLookupGenerationRef.current) {
          if (tombstone.directoryRefreshGeneration < refreshGeneration) {
            missingSessionLookupGenerationRef.current.delete(sessionId)
          }
        }
        const exactIds = [
          ...new Set([
            ...visibleSessionIdsRef.current,
            ...conversationsRef.current
              .filter(
                (conversation) =>
                  Boolean(conversation.parentId) &&
                  !conversation.draft &&
                  (conversation.status === 'idle' ||
                    conversation.status === 'working'),
              )
              .map((conversation) => conversation.id),
          ]),
        ]
        if (exactIds.length > 0) {
          const exactSet = new Set(exactIds)
          // A request issued before the connection break cannot be trusted to
          // represent the post-reconnect transcript. Cancel it explicitly;
          // the epoch below starts a fresh durable read even when the session
          // was already marked unhydrated.
          cancelHydrationRunsForSessions(
            exactIds,
            hydrationRunsRef.current,
            hydrationBuffersRef.current,
          )
          for (const sessionId of exactIds) {
            const retryTimer = hydrationRetryTimersRef.current.get(sessionId)
            if (retryTimer) clearTimeout(retryTimer)
            hydrationRetryTimersRef.current.delete(sessionId)
          }
          hydrationEpochRef.current += 1
          setHydrationEpoch(hydrationEpochRef.current)
          setConversations((current) =>
            current.map((conversation) =>
              exactSet.has(conversation.id) &&
              !conversation.draft &&
              conversation.hydrated
                ? { ...conversation, hydrated: false }
                : conversation,
            ),
          )
        }
        // Session triggers are at-least-once but not replayed. Re-list after
        // reconnect so terminal child states missed while offline replace the
        // cached working rows instead of leaving false-active chips behind.
        void (async () => {
          try {
            const metas = await listSessions()
            if (
              cancelled ||
              directoryRefreshGenerationRef.current !== refreshGeneration
            ) {
              return
            }
            const safeMetas = metas.filter((meta) => {
              const currentGeneration =
                sessionMetaLookupGenerationRef.current.get(meta.session_id) ?? 0
              const tombstone = missingSessionLookupGenerationRef.current.get(
                meta.session_id,
              )
              return shouldAcceptReconnectDirectoryRow({
                currentGeneration,
                missingGeneration: missingGenerationForDirectoryRefresh(
                  tombstone,
                  refreshGeneration,
                ),
              })
            })
            if (safeMetas.length > 0) {
              const listedIds = new Set(
                safeMetas.map((meta) => meta.session_id),
              )
              setMissingSessionIds((current) =>
                current.some((id) => listedIds.has(id))
                  ? current.filter((id) => !listedIds.has(id))
                  : current,
              )
            }
            setConversations((prev) =>
              mergeSessionListSnapshot(prev, safeMetas),
            )
          } catch (err) {
            if (import.meta.env.DEV) {
              console.warn('[conversations] session::list failed', err)
            }
          }
          if (
            cancelled ||
            directoryRefreshGenerationRef.current !== refreshGeneration
          ) {
            return
          }
          for (const sessionId of exactIds) {
            const requireWatched =
              (watchCountsRef.current.get(sessionId) ?? 0) > 0
            lookupSessionMeta(sessionId, {
              requireWatched,
              retries: 2,
            })
          }
        })()
      })
    })
    return () => {
      cancelled = true
      off?.()
    }
  }, [serverEnabled, lookupSessionMeta])

  /* ── Sidebar-level live events (all sessions) ─────────────────────────── */
  useEffect(() => {
    if (!serverEnabled) return
    let cancelled = false
    let off: (() => void) | null = null
    void getIiiClient().then((client) => {
      if (cancelled) return
      off = subscribeSessionDirectory(client, {
        onCreated: (event) => {
          clearConversationMissing(event.session_id)
          invalidateSessionMetaLookup(event.session_id)
          missingSessionLookupGenerationRef.current.delete(event.session_id)
          setConversations((prev) => {
            const existing = prev.find((c) => c.id === event.session_id)
            if (existing) {
              // The first send materialises the draft; flip it server-backed.
              return prev.map((conversation) => {
                if (
                  conversation.id !== event.session_id ||
                  !conversation.draft
                ) {
                  return conversation
                }
                return {
                  ...conversation,
                  draft: false,
                  serverMetaUpdatedAt: Math.max(
                    conversation.serverMetaUpdatedAt ?? -Infinity,
                    event.created_at,
                  ),
                  serverMetadataUpdatedAt: Math.max(
                    conversation.serverMetadataUpdatedAt ?? -Infinity,
                    event.created_at,
                  ),
                  serverStatusUpdatedAt: Math.max(
                    conversation.serverStatusUpdatedAt ?? -Infinity,
                    event.created_at,
                  ),
                }
              })
            }
            const stub: Conversation = {
              id: event.session_id,
              title: event.title || event.session_id,
              model: null,
              mode: DEFAULT_MODE,
              messages: [],
              status: event.status,
              serverMetaUpdatedAt: event.created_at,
              serverMetadataUpdatedAt: event.created_at,
              serverStatusUpdatedAt: event.created_at,
              hydrated: false,
              createdAt: event.created_at,
              updatedAt: event.created_at,
            }
            return [stub, ...prev]
          })
          // `session::created` carries no metadata (by contract), so a spawned
          // sub-agent / workflow node arrives here with no parent link and would
          // sit flat until a reload re-lists it. Fetch its meta once to learn
          // parent_session_id so it nests live in the sidebar tree.
          if (!cancelled) {
            const requireWatched =
              (watchCountsRef.current.get(event.session_id) ?? 0) > 0
            lookupSessionMeta(event.session_id, {
              requireWatched,
              retries: requireWatched ? 2 : 1,
            })
          }
        },
        onMetaUpdated: (event) => {
          clearConversationMissing(event.session_id)
          missingSessionLookupGenerationRef.current.delete(event.session_id)
          const known = conversationsRef.current.some(
            (conversation) => conversation.id === event.session_id,
          )
          patchConversation(event.session_id, (conversation) =>
            applyConversationMetadataEvent(conversation, event),
          )
          if (!known) {
            const requireWatched =
              (watchCountsRef.current.get(event.session_id) ?? 0) > 0
            lookupSessionMeta(event.session_id, {
              requireWatched,
              retries: requireWatched ? 2 : 1,
            })
          }
        },
        onStatusChanged: (event) => {
          clearConversationMissing(event.session_id)
          missingSessionLookupGenerationRef.current.delete(event.session_id)
          const known = conversationsRef.current.some(
            (conversation) => conversation.id === event.session_id,
          )
          patchConversation(event.session_id, (conversation) =>
            applyConversationStatusEvent(conversation, event),
          )
          if (!known) {
            const requireWatched =
              (watchCountsRef.current.get(event.session_id) ?? 0) > 0
            lookupSessionMeta(event.session_id, {
              requireWatched,
              retries: requireWatched ? 2 : 1,
            })
          }
        },
        onDeleted: (event) => {
          markConversationMissing(event.session_id)
          invalidateSessionMetaLookup(event.session_id)
          missingSessionLookupGenerationRef.current.set(event.session_id, {
            lookupGeneration:
              sessionMetaLookupGenerationRef.current.get(event.session_id) ?? 0,
            directoryRefreshGeneration: directoryRefreshGenerationRef.current,
          })
          cancelHydrationRunsForSessions(
            [event.session_id],
            hydrationRunsRef.current,
            hydrationBuffersRef.current,
          )
          transcriptSubscriptionsRef.current.get(event.session_id)?.()
          transcriptSubscriptionsRef.current.delete(event.session_id)
          transcriptSubscriptionEpochsRef.current.delete(event.session_id)
          const retryTimer = hydrationRetryTimersRef.current.get(
            event.session_id,
          )
          if (retryTimer) clearTimeout(retryTimer)
          hydrationRetryTimersRef.current.delete(event.session_id)
          setConversations((list) =>
            list.filter((c) => c.id !== event.session_id),
          )
          revisionsRef.current.delete(event.session_id)
          setActiveId((current) =>
            current === event.session_id ? null : current,
          )
        },
      })
    })
    return () => {
      cancelled = true
      off?.()
    }
  }, [
    serverEnabled,
    clearConversationMissing,
    markConversationMissing,
    patchConversation,
    invalidateSessionMetaLookup,
    lookupSessionMeta,
  ])

  /* ── Visible-session transcripts: live reconcile + hydration ──────────── */
  // Every mounted ChatPanel registers explicitly. Keeping the global active
  // id here would leave a transcript subscription alive after the final chat
  // panel closes.
  const watchedIds = useMemo(
    () => new Set(watchedSessionIds),
    [watchedSessionIds],
  )
  const watchedIdsSignature = JSON.stringify([...watchedIds].sort())
  const serverWatchedIds = useMemo(
    () =>
      conversations
        .filter(
          (conversation) =>
            watchedIds.has(conversation.id) && !conversation.draft,
        )
        .map((conversation) => conversation.id)
        .sort(),
    [conversations, watchedIds],
  )
  const serverWatchedSignature = JSON.stringify(serverWatchedIds)
  visibleSessionIdsRef.current = [...watchedIds].sort()

  useEffect(() => {
    // Per-session epochs live in a ref; the revision intentionally wakes this
    // reconciliation after a same-signature unwatch -> watch transition.
    void watchLifecycleRevision
    const subscriptions = transcriptSubscriptionsRef.current
    const wanted = new Set(
      serverEnabled ? sessionIdsFromSignature(serverWatchedSignature) : [],
    )
    for (const [sessionId, off] of subscriptions) {
      if (
        wanted.has(sessionId) &&
        transcriptSubscriptionEpochsRef.current.get(sessionId) ===
          (watchLifecycleEpochsRef.current.get(sessionId) ?? 0)
      ) {
        continue
      }
      off()
      subscriptions.delete(sessionId)
      transcriptSubscriptionEpochsRef.current.delete(sessionId)
    }
    if (!serverEnabled || wanted.size === 0) return

    let cancelled = false
    const revisionsFor = (sessionId: string) => {
      let revisions = revisionsRef.current.get(sessionId)
      if (!revisions) {
        revisions = new Map()
        revisionsRef.current.set(sessionId, revisions)
      }
      return revisions
    }

    void getIiiClient().then((client) => {
      if (cancelled) return
      for (const sessionId of wanted) {
        if (subscriptions.has(sessionId)) continue
        const watchEpoch = watchLifecycleEpochsRef.current.get(sessionId) ?? 0
        const off = subscribeSessionTranscript(client, sessionId, {
          onMessageAdded: (event) => {
            const item = {
              entry_id: event.entry_id,
              message: event.message,
              custom: event.custom,
              origin: event.origin,
            }
            hydrationBuffersRef.current
              .get(sessionId)
              ?.push({ item, updated: false })
            patchConversation(sessionId, (conversation) =>
              markDurableStarted(
                {
                  ...conversation,
                  messages: applyEntryUpsert(conversation.messages, item, {
                    sessionId,
                    working: conversation.status === 'working',
                  }),
                  updatedAt: event.timestamp,
                },
                item.message?.role === 'assistant',
              ),
            )
          },
          onMessageUpdated: (event) => {
            const revisions = revisionsFor(sessionId)
            const previous = revisions.get(event.entry_id) ?? -1
            if (event.revision <= previous) return
            revisions.set(event.entry_id, event.revision)
            const item = {
              entry_id: event.entry_id,
              message: event.message,
              origin: event.origin,
            }
            hydrationBuffersRef.current
              .get(sessionId)
              ?.push({ item, updated: true })
            patchConversation(sessionId, (conversation) =>
              markDurableStarted(
                {
                  ...conversation,
                  messages: applyEntryUpsert(conversation.messages, item, {
                    sessionId,
                    streaming: conversation.status === 'working',
                    working: conversation.status === 'working',
                  }),
                  updatedAt: event.timestamp,
                },
                item.message?.role === 'assistant',
              ),
            )
          },
        })
        if (
          cancelled ||
          !wanted.has(sessionId) ||
          (watchCountsRef.current.get(sessionId) ?? 0) === 0 ||
          (watchLifecycleEpochsRef.current.get(sessionId) ?? 0) !== watchEpoch
        )
          off()
        else {
          subscriptions.set(sessionId, off)
          transcriptSubscriptionEpochsRef.current.set(sessionId, watchEpoch)
        }
      }
    })
    return () => {
      cancelled = true
    }
  }, [
    serverEnabled,
    serverWatchedSignature,
    watchLifecycleRevision,
    patchConversation,
  ])

  const hydrationTargetIds = serverWatchedIds.filter((sessionId) =>
    conversations.some(
      (conversation) => conversation.id === sessionId && !conversation.hydrated,
    ),
  )
  const hydrationTargetSignature = JSON.stringify(hydrationTargetIds)
  useEffect(() => {
    // See the transcript subscription effect above.
    void watchLifecycleRevision
    const runs = hydrationRunsRef.current
    const wanted = new Set(
      serverEnabled ? sessionIdsFromSignature(serverWatchedSignature) : [],
    )
    for (const [sessionId, run] of runs) {
      if (
        wanted.has(sessionId) &&
        run.watchEpoch === (watchLifecycleEpochsRef.current.get(sessionId) ?? 0)
      ) {
        continue
      }
      run.cancelled = true
      runs.delete(sessionId)
      hydrationBuffersRef.current.delete(sessionId)
    }
    if (!serverEnabled) return

    for (const sessionId of sessionIdsFromSignature(hydrationTargetSignature)) {
      if (runs.has(sessionId)) continue
      // One buffer per session lets sibling panels hydrate concurrently while
      // preserving live revisions that race either durable read.
      const upserts: HydrationUpsert[] = []
      const watchEpoch = watchLifecycleEpochsRef.current.get(sessionId) ?? 0
      const run: HydrationRun = {
        cancelled: false,
        connectionEpoch: hydrationEpoch,
        watchEpoch,
        upserts,
      }
      runs.set(sessionId, run)
      hydrationBuffersRef.current.set(sessionId, upserts)
      void fetchTranscript(sessionId)
        .then((items) => {
          if (
            run.cancelled ||
            run.connectionEpoch !== hydrationEpochRef.current ||
            run.watchEpoch !==
              (watchLifecycleEpochsRef.current.get(sessionId) ?? 0)
          ) {
            return
          }
          patchConversation(sessionId, (conversation) =>
            mergeHydratedConversation(conversation, items, upserts),
          )
          const retryTimer = hydrationRetryTimersRef.current.get(sessionId)
          if (retryTimer) clearTimeout(retryTimer)
          hydrationRetryTimersRef.current.delete(sessionId)
        })
        .catch((err) => {
          if (
            run.cancelled ||
            run.connectionEpoch !== hydrationEpochRef.current ||
            run.watchEpoch !==
              (watchLifecycleEpochsRef.current.get(sessionId) ?? 0)
          ) {
            return
          }
          patchConversation(sessionId, completeFailedHydration)
          if (
            (watchCountsRef.current.get(sessionId) ?? 0) > 0 &&
            !hydrationRetryTimersRef.current.has(sessionId)
          ) {
            const retryTimer = setTimeout(() => {
              hydrationRetryTimersRef.current.delete(sessionId)
              if ((watchCountsRef.current.get(sessionId) ?? 0) === 0) return
              patchConversation(sessionId, (conversation) =>
                conversation.hydrated
                  ? { ...conversation, hydrated: false }
                  : conversation,
              )
            }, 2_000)
            hydrationRetryTimersRef.current.set(sessionId, retryTimer)
          }
          if (import.meta.env.DEV) {
            console.warn(
              '[conversations] transcript hydration failed',
              sessionId,
              err,
            )
          }
        })
        .finally(() => {
          if (runs.get(sessionId) === run) runs.delete(sessionId)
          if (hydrationBuffersRef.current.get(sessionId) === upserts) {
            hydrationBuffersRef.current.delete(sessionId)
          }
        })
    }
  }, [
    serverEnabled,
    serverWatchedSignature,
    hydrationTargetSignature,
    hydrationEpoch,
    watchLifecycleRevision,
    patchConversation,
  ])

  useEffect(
    () => () => {
      for (const off of transcriptSubscriptionsRef.current.values()) off()
      transcriptSubscriptionsRef.current.clear()
      transcriptSubscriptionEpochsRef.current.clear()
      watchLifecycleEpochsRef.current.clear()
      for (const run of hydrationRunsRef.current.values()) run.cancelled = true
      hydrationRunsRef.current.clear()
      hydrationBuffersRef.current.clear()
      for (const timer of hydrationRetryTimersRef.current.values()) {
        clearTimeout(timer)
      }
      hydrationRetryTimersRef.current.clear()
      for (const timer of sessionMetaLookupTimersRef.current.values()) {
        clearTimeout(timer)
      }
      sessionMetaLookupTimersRef.current.clear()
      for (const [
        sessionId,
        generation,
      ] of sessionMetaLookupGenerationRef.current) {
        sessionMetaLookupGenerationRef.current.set(sessionId, generation + 1)
      }
      missingSessionLookupGenerationRef.current.clear()
    },
    [],
  )

  /* Sessions without a mounted panel receive no transcript events. Mark them
     stale when visibility changes so reopening always re-reads durable truth. */
  useEffect(() => {
    if (!serverEnabled) return
    setConversations((previous) =>
      markUnwatchedStale(
        previous,
        new Set(sessionIdsFromSignature(watchedIdsSignature)),
      ),
    )
  }, [serverEnabled, watchedIdsSignature])

  /* Migrate model ids once catalog-backed keys are known (local-only; the
     server metadata is rewritten on the next explicit model change). Gated
     on catalogReady so a stale placeholder catalog can't clobber picks. */
  useEffect(() => {
    if (!catalogSig) return
    if (catalogReady === false) return
    const keys = catalogSig.split('\u0001')
    const valid = new Set(keys)
    const fallback = keys[0]
    setConversations((prev) => {
      return applyCatalogModelFallback(prev, valid, fallback)
    })
    const lastModel = loadLastModel()
    if (lastModel && !valid.has(lastModel)) {
      saveLastModel(fallback)
    }
  }, [catalogSig, catalogReady])

  useEffect(() => {
    saveActiveId(activeId)
  }, [activeId])

  /* Ensure there's always a sensible "active" pointer at the start. */
  useEffect(() => {
    const next = resolveActiveConversationId({
      conversationIds: conversations.map((c) => c.id),
      activeId,
      pendingSelectId: pendingSelectIdRef.current,
    })
    pendingSelectIdRef.current = next.pendingSelectId
    if (next.activeId !== activeId) setActiveId(next.activeId)
  }, [conversations, activeId])

  const active = useMemo(
    () => conversations.find((c) => c.id === activeId) ?? null,
    [conversations, activeId],
  )
  const missingConversationIds = useMemo(
    () => new Set(missingSessionIds),
    [missingSessionIds],
  )

  const createNew = useCallback(() => {
    // Asking for a new chat while an untouched one is already open reads as
    // "nothing happened": the second empty draft is indistinguishable from
    // the first, and they pile up in the list. Hand back the one in front of
    // you instead, and put the caret in it.
    const current = conversations.find(
      (conversation) => conversation.id === activeId,
    )
    if (current && isUntouchedDraft(current)) {
      requestComposerFocus()
      return current.id
    }
    const next = emptyConversation(
      loadLastModel(),
      loadLastThinkingLevel() ?? DEFAULT_THINKING_LEVEL,
    )
    setConversations((list) => [next, ...list])
    setActiveId(next.id)
    return next.id
  }, [conversations, activeId])

  const select = useCallback((id: string) => {
    pendingSelectIdRef.current = id
    setActiveId(id)
  }, [])

  const bumpWatchLifecycle = useCallback((id: string) => {
    bumpSessionWatchEpoch(watchLifecycleEpochsRef.current, id)
    setWatchLifecycleRevision((revision) => revision + 1)
  }, [])

  const watchConversation = useCallback(
    (rawId: string) => {
      const id = rawId.trim()
      if (!id) return () => {}
      const counts = watchCountsRef.current
      const previousCount = counts.get(id) ?? 0
      counts.set(id, previousCount + 1)
      if (previousCount === 0) {
        bumpWatchLifecycle(id)
        setWatchedSessionIds((current) =>
          current.includes(id) ? current : [...current, id],
        )
        if (
          serverEnabled &&
          !conversationsRef.current.some(
            (conversation) => conversation.id === id,
          )
        ) {
          // A persisted panel can deep-link to a session outside the bounded
          // directory list. Resolve it directly instead of leaving the panel
          // blank forever.
          lookupSessionMeta(id, { requireWatched: true, retries: 2 })
        }
      }

      let released = false
      return () => {
        if (released) return
        released = true
        const currentCount = counts.get(id) ?? 0
        if (currentCount > 1) {
          counts.set(id, currentCount - 1)
          return
        }
        counts.delete(id)
        bumpWatchLifecycle(id)
        invalidateSessionMetaLookup(id)
        cancelHydrationRunsForSessions(
          [id],
          hydrationRunsRef.current,
          hydrationBuffersRef.current,
        )
        transcriptSubscriptionsRef.current.get(id)?.()
        transcriptSubscriptionsRef.current.delete(id)
        transcriptSubscriptionEpochsRef.current.delete(id)
        const retryTimer = hydrationRetryTimersRef.current.get(id)
        if (retryTimer) clearTimeout(retryTimer)
        hydrationRetryTimersRef.current.delete(id)
        patchConversation(id, (conversation) =>
          !conversation.draft && conversation.hydrated
            ? { ...conversation, hydrated: false }
            : conversation,
        )
        setWatchedSessionIds((current) =>
          current.filter((sessionId) => sessionId !== id),
        )
      }
    },
    [
      serverEnabled,
      bumpWatchLifecycle,
      invalidateSessionMetaLookup,
      lookupSessionMeta,
      patchConversation,
    ],
  )

  const rename = useCallback(
    (id: string, title: string) => {
      const trimmed = title.trim()
      patchConversation(id, (c) =>
        applyConversationMetadataPatch(c, {
          title: trimmed || c.title,
          titleManual: true,
        }),
      )
      if (!serverEnabled || !trimmed) return
      const conv = conversations.find((c) => c.id === id)
      if (!conv || conv.draft) return
      const updated = applyConversationMetadataPatch(conv, {
        title: trimmed,
        titleManual: true,
      })
      void setSessionMeta({
        session_id: id,
        title: trimmed,
        metadata: metadataForWrite(updated),
      }).catch((err) => {
        if (import.meta.env.DEV)
          console.warn('[conversations] rename failed', err)
      })
    },
    [patchConversation, serverEnabled, conversations],
  )

  const remove = useCallback(
    (id: string) => {
      const conv = conversations.find((c) => c.id === id)
      setConversations((list) => list.filter((c) => c.id !== id))
      markConversationMissing(id)
      invalidateSessionMetaLookup(id)
      missingSessionLookupGenerationRef.current.set(id, {
        lookupGeneration: sessionMetaLookupGenerationRef.current.get(id) ?? 0,
        directoryRefreshGeneration: directoryRefreshGenerationRef.current,
      })
      cancelHydrationRunsForSessions(
        [id],
        hydrationRunsRef.current,
        hydrationBuffersRef.current,
      )
      transcriptSubscriptionsRef.current.get(id)?.()
      transcriptSubscriptionsRef.current.delete(id)
      transcriptSubscriptionEpochsRef.current.delete(id)
      const retryTimer = hydrationRetryTimersRef.current.get(id)
      if (retryTimer) clearTimeout(retryTimer)
      hydrationRetryTimersRef.current.delete(id)
      revisionsRef.current.delete(id)
      draftTextsRef.current.delete(id)
      lastSavedDraftRef.current.delete(id)
      if (pendingDraftRef.current?.id === id) pendingDraftRef.current = null
      setActiveId((current) => (current === id ? null : current))
      // Closing the conversation orphans any worktree claim this console
      // flow made for it; release best-effort (no-op for other claims).
      void releaseConsoleClaimIfAny(id)
      if (!serverEnabled || !conv || conv.draft) return
      void deleteSession(id).catch((err) => {
        if (import.meta.env.DEV)
          console.warn('[conversations] delete failed', err)
      })
    },
    [
      serverEnabled,
      conversations,
      invalidateSessionMetaLookup,
      markConversationMissing,
    ],
  )

  const writeMeta = useCallback(
    (conv: Conversation) => {
      if (!serverEnabled || conv.draft) return
      void setSessionMeta({
        session_id: conv.id,
        metadata: metadataForWrite(conv),
      }).catch((err) => {
        if (import.meta.env.DEV)
          console.warn('[conversations] set_meta failed', err)
      })
    },
    [serverEnabled],
  )

  const setModel = useCallback(
    (id: string, model: ModelId) => {
      patchConversation(id, (c) => applyConversationMetadataPatch(c, { model }))
      saveLastModel(model)
      const conv = conversations.find((c) => c.id === id)
      if (conv) writeMeta(applyConversationMetadataPatch(conv, { model }))
    },
    [patchConversation, conversations, writeMeta],
  )

  const setThinkingLevel = useCallback(
    (id: string, thinkingLevel: ThinkingLevel) => {
      patchConversation(id, (c) =>
        applyConversationMetadataPatch(c, { thinkingLevel }),
      )
      saveLastThinkingLevel(thinkingLevel)
      const conv = conversations.find((c) => c.id === id)
      if (conv)
        writeMeta(applyConversationMetadataPatch(conv, { thinkingLevel }))
    },
    [patchConversation, conversations, writeMeta],
  )

  const setMode = useCallback(
    (id: string, mode: Mode) => {
      patchConversation(id, (c) => applyConversationMetadataPatch(c, { mode }))
      const conv = conversations.find((c) => c.id === id)
      if (conv) writeMeta(applyConversationMetadataPatch(conv, { mode }))
    },
    [patchConversation, conversations, writeMeta],
  )

  const setMemoryBank = useCallback(
    (id: string, memoryBank: string | null) => {
      patchConversation(id, (c) =>
        applyConversationMetadataPatch(c, { memoryBank }),
      )
      const conv = conversations.find((c) => c.id === id)
      if (conv) writeMeta(applyConversationMetadataPatch(conv, { memoryBank }))
    },
    [patchConversation, conversations, writeMeta],
  )

  const setSystemPrompt = useCallback(
    (id: string, systemPrompt: SystemPromptState) => {
      patchConversation(id, (c) =>
        applyConversationMetadataPatch(c, { systemPrompt }),
      )
      const conv = conversations.find((c) => c.id === id)
      if (conv)
        writeMeta(applyConversationMetadataPatch(conv, { systemPrompt }))
    },
    [patchConversation, conversations, writeMeta],
  )

  const setSkills = useCallback(
    (id: string, skills: string[] | undefined) => {
      const normalized = skills?.length ? skills : undefined
      patchConversation(id, (c) =>
        applyConversationMetadataPatch(c, { skills: normalized }),
      )
      const conv = conversations.find((c) => c.id === id)
      if (conv)
        writeMeta(applyConversationMetadataPatch(conv, { skills: normalized }))
    },
    [patchConversation, conversations, writeMeta],
  )

  const setWorkingDir = useCallback(
    (id: string, dir: string | null) => {
      patchConversation(id, (c) =>
        applyConversationMetadataPatch(c, { workingDir: dir }),
      )
      if (dir) saveRecentProject(dir)
      // Moving the working directory away from a console-claimed worktree
      // releases the claim (keepPath guards the pick-this-worktree flow,
      // which records the claim before updating the dir).
      void releaseConsoleClaimIfAny(id, { keepPath: dir })
      const conv = conversations.find((c) => c.id === id)
      if (conv)
        writeMeta(applyConversationMetadataPatch(conv, { workingDir: dir }))
    },
    [patchConversation, conversations, writeMeta],
  )

  const prefillWorkingDir = useCallback(
    (id: string, dir: string) => {
      patchConversation(id, (c) =>
        c.draft && c.workingDir == null ? { ...c, workingDir: dir } : c,
      )
    },
    [patchConversation],
  )

  const appendMessage = useCallback(
    (id: string, message: Message) =>
      patchConversation(id, (c) => appendMessageToConversation(c, message)),
    [patchConversation],
  )

  const updateMessage = useCallback(
    (id: string, messageId: string, patch: MessagePatch) =>
      patchConversation(id, (c) => ({
        ...c,
        messages: c.messages.map((m) =>
          m.id === messageId ? ({ ...m, ...patch } as Message) : m,
        ),
        updatedAt: Date.now(),
      })),
    [patchConversation],
  )

  const compactConversation = useCallback(
    (id: string, marker: Message) =>
      patchConversation(id, (c) => ({
        ...c,
        messages: [marker],
        updatedAt: Date.now(),
      })),
    [patchConversation],
  )

  const ensureSession = useCallback(
    async (id: string, titleHint?: string) => {
      const conv = conversations.find((c) => c.id === id)
      if (!serverEnabled || !conv) return
      const metaUpdate = preSendMetaUpdate(conv)
      if (metaUpdate) {
        const pendingEdits = conv.legacySkillMigration?.edits
        try {
          await setSessionMeta(metaUpdate)
          patchConversation(id, (current) =>
            completePreSendMetaUpdate(current, pendingEdits),
          )
        } catch (err) {
          if (import.meta.env.DEV) {
            console.warn('[conversations] session::set-meta failed', err)
          }
          throw err
        }
        return
      }
      if (!conv.draft) return
      const title = conv.titleManual
        ? conv.title
        : titleHint
          ? deriveTitle(titleHint)
          : conv.title
      try {
        const resp = await ensureSessionApi({
          session_id: id,
          title,
          metadata: metadataFor(conv),
        })
        patchConversation(id, (c) => ({
          ...mergeConversationMeta(
            { ...c, draft: false, hydrated: false },
            resp.meta,
          ),
          draft: false,
          // Leave the session un-hydrated: the transcript subscription only
          // mounts after this patch re-renders, so the first server events
          // can fire before the binding exists (at-most-once, no replay).
          // The hydration read-back then recovers anything missed; it folds
          // through applyEntryUpsert, so the optimistic user row and any
          // events that race the fetch survive.
          hydrated: false,
        }))
      } catch (err) {
        if (import.meta.env.DEV) {
          console.warn('[conversations] session::ensure failed', err)
        }
        throw err
      }
    },
    [serverEnabled, conversations, patchConversation],
  )

  /* Live mirror for the draft callbacks: they fire from debounce timers and
     editor events, where a stale `conversations` closure would misclassify a
     just-materialised session as still-local. */
  const flushDraft = useCallback(() => {
    if (draftTimerRef.current) {
      clearTimeout(draftTimerRef.current)
      draftTimerRef.current = null
    }
    const pending = pendingDraftRef.current
    pendingDraftRef.current = null
    if (!pending) return
    if (lastSavedDraftRef.current.get(pending.id) === pending.text) return
    const chain = draftSaveChainRef.current
    const tail = (chain.get(pending.id) ?? Promise.resolve())
      .then(async () => {
        // Re-check under the chain: an earlier link may have saved this very
        // value already. The saved-marker moves only AFTER the RPC resolves —
        // a failed save stays eligible for retry on the next flush.
        if (lastSavedDraftRef.current.get(pending.id) === pending.text) return
        await setSessionDraft(pending.id, pending.text || null)
        lastSavedDraftRef.current.set(pending.id, pending.text)
      })
      .catch((err) => {
        if (import.meta.env.DEV) {
          console.warn('[conversations] set-draft failed', err)
        }
      })
      .finally(() => {
        if (chain.get(pending.id) === tail) chain.delete(pending.id)
      })
    chain.set(pending.id, tail)
  }, [])

  const setDraftText = useCallback(
    (id: string, text: string) => {
      draftTextsRef.current.set(id, text)
      if (!serverEnabled) return
      const conv = conversationsRef.current.find((c) => c.id === id)
      // Local drafts have no session yet; their text still lives in the ref
      // map so in-tab switches keep it.
      if (!conv || conv.draft) return
      if (pendingDraftRef.current && pendingDraftRef.current.id !== id) {
        flushDraft()
      }
      pendingDraftRef.current = { id, text }
      if (draftTimerRef.current) clearTimeout(draftTimerRef.current)
      draftTimerRef.current = setTimeout(flushDraft, DRAFT_SAVE_DEBOUNCE_MS)
    },
    [serverEnabled, flushDraft],
  )

  const getDraftText = useCallback((id: string): string | undefined => {
    const live = draftTextsRef.current.get(id)
    if (live !== undefined) return live || undefined
    return conversationsRef.current.find((c) => c.id === id)?.draftText
  }, [])

  /* A hidden tab may be a refresh in progress — flush the pending save so
     the debounce window doesn't swallow the last keystrokes. */
  useEffect(() => {
    if (!serverEnabled || typeof document === 'undefined') return
    const onVisibilityChange = () => {
      if (document.visibilityState === 'hidden') flushDraft()
    }
    document.addEventListener('visibilitychange', onVisibilityChange)
    return () =>
      document.removeEventListener('visibilitychange', onVisibilityChange)
  }, [serverEnabled, flushDraft])

  return {
    conversations,
    activeId,
    active,
    connectionState,
    missingConversationIds,
    createNew,
    select,
    watchConversation,
    rename,
    remove,
    setModel,
    setThinkingLevel,
    setMemoryBank,
    setSystemPrompt,
    setSkills,
    setMode,
    setWorkingDir,
    prefillWorkingDir,
    appendMessage,
    updateMessage,
    compactConversation,
    ensureSession,
    setDraftText,
    getDraftText,
  }
}

export { uid }
