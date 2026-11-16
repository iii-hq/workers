const lines = (text: string): string[] =>
  text === '' ? [] : text.replace(/\n$/, '').split('\n')

/**
 * Line-level three-way merge for a rule save the worker refused as
 * `rule_changed`: `base` is what the editor loaded, `mine` the draft,
 * `theirs` what the rule holds now. The draft wins; a line gone from disk
 * since `base` (demoted, removed by hand) stays gone, and a line new on disk
 * (a learned correction, a promotion) is appended. A line disk moved later
 * (restated, so now the newest) moves to the end too, so the cap does not
 * demote it first. A plain union would put demoted lines back.
 */
export function mergeRule(base: string, mine: string, theirs: string): string {
  const baseAt = new Map(lines(base).map((line, i) => [line, i]))
  const theirLines = lines(theirs)
  const stillThere = new Set(theirLines)
  const kept = lines(mine).filter(
    (line) => line.trim() === '' || !baseAt.has(line) || stillThere.has(line),
  )
  const keptLines = new Set(kept)
  // Lines that go last, in disk's order.
  const tail = new Set<string>()
  let furthest = -1
  for (const line of theirLines) {
    if (line.trim() === '') continue
    const at = baseAt.get(line)
    if (at === undefined) {
      if (!keptLines.has(line)) tail.add(line) // new on disk
    } else if (at < furthest) {
      if (keptLines.has(line)) tail.add(line) // moved past a later base line
    } else furthest = at
  }
  return [
    ...kept.filter((line) => !tail.has(line)),
    ...theirLines.filter((line) => tail.has(line)),
  ]
    .map((line) => `${line}\n`)
    .join('')
}

/**
 * The base for the editor's next save once `sent` was stored as `saved`
 * while the editor came to hold `now`. Text typed during the save was edited
 * from `sent`, so its next save must be refused and merged with `saved`;
 * taking `saved` as its base would let it overwrite lines that save merged
 * in or put back lines the cap demoted.
 */
export const baseAfterSave = (sent: string, now: string, saved: string) =>
  now === sent ? saved : sent
