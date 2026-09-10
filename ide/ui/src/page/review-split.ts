export type WholeFileChange = 'added' | 'deleted'

/**
 * A file that exists on only one side has nothing to show in the other split
 * column; the caller renders that column as a placeholder instead of letting
 * the diff collapse into the unified layout.
 */
export function wholeFileChange(
  oldContents: string,
  newContents: string,
): WholeFileChange | null {
  const oldEmpty = oldContents.length === 0
  const newEmpty = newContents.length === 0
  if (oldEmpty === newEmpty) return null
  return oldEmpty ? 'added' : 'deleted'
}

export function wholeFileLabel(
  change: WholeFileChange,
  lines: number,
): { title: string; detail: string } {
  const count = `${lines} ${lines === 1 ? 'line' : 'lines'}`
  return change === 'deleted'
    ? { title: 'File deleted', detail: `${count} removed` }
    : { title: 'New file', detail: `${count} added` }
}
