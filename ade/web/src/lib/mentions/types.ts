/**
 * Worker-defined chat mentions — the console side of `crates/mention-contract`.
 *
 * A worker declares a mention provider in the registration metadata of its
 * get function (`metadata.mention`); the descriptor names the search
 * function. In text a mention is `@<name>(id="<id>")`. The view shapes are
 * part of the injectable-UI contract (a worker's preview renderer receives
 * them), so they live in `@/types/injectable-ui`.
 */

export type {
  MentionField,
  MentionOpen,
  MentionView,
} from '@/types/injectable-ui'

/** The descriptor a worker registers under `metadata.mention`. */
export interface MentionProviderDescriptor {
  v: number
  /** Token name: `@<name>(id="…")`. */
  name: string
  /** Plural noun for the menu group header ("Tickets"). */
  label: string
  description?: string
  icon?: string
  color?: string
  /** Function id of the search function. */
  search: string
  /** The domain function an agent reads the full item through. */
  details?: { function_id: string; id_field: string }
}

/** A provider the console can use: the descriptor plus where it lives. */
export interface MentionProvider extends MentionProviderDescriptor {
  /** Function id of the get function (the one carrying the descriptor). */
  getFunctionId: string
  /** Worker that registered it, when the engine says. */
  workerName?: string
}

/** One search row. */
export interface MentionItem {
  id: string
  label: string
  hint?: string
  description?: string
  icon?: string
  color?: string
}

/** One mention found in text. */
export interface MentionRef {
  name: string
  id: string
  /** The token exactly as written. */
  token: string
}

/** Where a search was asked from. */
export interface MentionSearchContext {
  session_id?: string
  working_dir?: string
}
