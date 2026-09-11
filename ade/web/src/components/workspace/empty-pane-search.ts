import type { ScreenOption } from './use-screen-options'

function normalized(value: string): string {
  return value
    .normalize('NFKD')
    .replace(/\p{Diacritic}/gu, '')
    .toLowerCase()
}

/**
 * Rank a page against a query. Prefixes win, then contained text, then a
 * loose subsequence (`wkr` → `workers`). Metadata participates in exact
 * matching so injected page ids and descriptions remain searchable.
 */
function optionScore(option: ScreenOption, query: string): number {
  const label = normalized(option.label)
  const metadata = normalized(
    [option.description, option.value, ...(option.keywords ?? [])]
      .filter(Boolean)
      .join(' '),
  )

  if (label === query) return 0
  if (label.startsWith(query)) return 1
  if (label.includes(query)) return 2
  if (metadata.includes(query)) return 3

  let queryIndex = 0
  for (const character of label) {
    if (character === query[queryIndex]) queryIndex += 1
    if (queryIndex === query.length) return 4
  }
  return Number.POSITIVE_INFINITY
}

export function filterScreenOptions(
  screenOptions: ScreenOption[],
  query: string,
): ScreenOption[] {
  const normalizedQuery = normalized(query.trim())
  if (!normalizedQuery) return screenOptions

  return screenOptions
    .map((option, order) => ({
      option,
      order,
      score: optionScore(option, normalizedQuery),
    }))
    .filter(({ score }) => Number.isFinite(score))
    .sort((left, right) => left.score - right.score || left.order - right.order)
    .map(({ option }) => option)
}
