/** Types shared by the SDK's modules and the generated props. */

import type { KnownIcon, StepState, Tone } from './generated/catalog'

export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue }
export type JsonObject = { [key: string]: JsonValue }
export type Unsubscribe = () => void

/**
 * A Lucide icon name in kebab-case (`globe`, `refresh-cw`): one of the
 * catalogue's `icons` (offered by the editor), or any string, which a host
 * without that icon draws as a neutral square.
 */
export type IconName = KnownIcon | (string & {})

/** A Chart's series: one array of numbers, or one array per series; null is a gap. */
export type ChartSeries = readonly (number | null)[] | readonly (readonly (number | null)[])[]

/** A dashed horizontal line of a Chart. */
export interface ChartThreshold {
  value: number
  /** Default muted. */
  tone?: Tone
  label?: string
}

/** One step of a Steps. */
export interface StepItem {
  title: string
  detail?: string
  /** Default pending. */
  state?: StepState
  /** 0..1: a thin bar under an active step. */
  progress?: number
}

/** CodeView lines to mark, numbered as shown: `[12, [20, 24]]` marks 12 and 20 to 24. */
export type LineMarks = readonly (number | readonly [number, number])[]

/** All sides, `[vertical, horizontal]` or `[top, end, bottom, start]`, in dp. */
export type Padding = number | readonly [number, number] | readonly [number, number, number, number]

/**
 * A prop value bound to a field of the item a List template row draws
 * (`{"$item": "field.path"}` on the wire, design §5.5). Read a field of the
 * template's `row` argument to get one.
 */
export interface Binding<T = unknown> {
  readonly $item: string
  /** Type-only: the field's type. */
  readonly __value?: T
}

/** The `row` a List template receives: every field, at any depth, is a {@link Binding}. */
export type Bound<T> = {
  readonly [K in keyof T]-?: Binding<T[K]> &
    (NonNullable<T[K]> extends readonly unknown[]
      ? unknown
      : NonNullable<T[K]> extends object
        ? Bound<NonNullable<T[K]>>
        : unknown)
}

/** What every prop accepts: its value, a template binding, or null/undefined for the default. */
export type Prop<T> = T | Binding | null | undefined

/** A node of the tree (what a component's `ref` gives): only an anchor and a focus target. */
export interface NodeRef {
  readonly id: number
}
