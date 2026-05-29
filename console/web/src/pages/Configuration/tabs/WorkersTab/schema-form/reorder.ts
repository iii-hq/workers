/**
 * Pure array reorder used by the drag-to-reorder affordance in
 * `ArrayField` and `DictionaryField`. Returns a new array (never mutates
 * the input) so it slots straight into React state updates.
 */
export function moveItem<T>(list: readonly T[], from: number, to: number): T[] {
  const copy = list.slice()
  if (
    from === to ||
    from < 0 ||
    to < 0 ||
    from >= copy.length ||
    to >= copy.length
  ) {
    return copy
  }
  const [item] = copy.splice(from, 1)
  copy.splice(to, 0, item)
  return copy
}
