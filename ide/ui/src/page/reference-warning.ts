/** Never imply a citation selected its original source: we open the live buffer. */
export function referenceWarning(
  reveal: { line: number; endLine?: number; column?: number },
  content: string,
  readOnly: 'binary' | 'truncated' | null,
): string | null {
  if (readOnly === 'binary') return 'This file is binary; line references are not available.'
  const lines = content.split('\n')
  if (reveal.line > lines.length || (reveal.endLine ?? reveal.line) > lines.length) {
    return readOnly === 'truncated'
      ? `The reference is outside the loaded preview (${lines.length} lines). The file has not been loaded in full.`
      : `The reference exceeds the current file (${lines.length} lines). It may have changed since the message was written.`
  }
  if (reveal.column !== undefined && reveal.column > lines[reveal.line - 1].length + 1) {
    return 'The referenced column is outside the current line. The file may have changed.'
  }
  return null
}
