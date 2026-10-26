/** Types shared by the SDK's modules and the generated props. */

export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue }
export type JsonObject = { [key: string]: JsonValue }
export type Unsubscribe = () => void

/** A Lucide icon name in kebab-case (`globe`, `refresh-cw`); an unknown name draws a neutral square. */
export type IconName = string

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
