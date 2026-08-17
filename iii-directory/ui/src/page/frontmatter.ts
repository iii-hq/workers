export interface FrontmatterField {
  key: string
  value: string
  present: boolean
  bare?: boolean
  raw?: string
}

interface FrontmatterParts {
  body: string
  eol: '\n' | '\r\n'
  hasFrontmatter: boolean
  yaml: string
}

function splitFrontmatter(content: string): FrontmatterParts {
  const eol = content.startsWith('---\r\n') ? '\r\n' : content.startsWith('---\n') ? '\n' : null
  if (!eol) {
    return { body: content, eol: '\n', hasFrontmatter: false, yaml: '' }
  }

  const rest = content.slice(3 + eol.length)
  const close = /\r?\n---(?:\r?\n|$)/.exec(rest)
  if (!close || close.index === undefined) {
    return { body: content, eol, hasFrontmatter: false, yaml: '' }
  }

  return {
    body: rest.slice(close.index + close[0].length),
    eol,
    hasFrontmatter: true,
    yaml: rest.slice(0, close.index),
  }
}

function joinFrontmatter(parts: FrontmatterParts): string {
  if (!parts.hasFrontmatter) return parts.body
  const body = parts.body ? `${parts.eol}${parts.body}` : ''
  return `---${parts.eol}${parts.yaml}${parts.eol}---${body}`
}

function fieldRange(lines: string[], key: string): [number, number] | null {
  const escaped = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const header = new RegExp(`^${escaped}\\s*:(.*)$`)
  const start = lines.findIndex((line) => header.test(line))
  if (start === -1) return null

  const raw = header.exec(lines[start])?.[1].trimStart() ?? ''
  let end = start + 1
  if (/^[>|]/.test(raw)) {
    while (end < lines.length && (lines[end].trim() === '' || /^[ \t]/.test(lines[end]))) {
      end += 1
    }
  }
  return [start, end]
}

function decodeScalar(raw: string, continuation: string[]): string {
  const value = raw.trim()
  if (/^[>|]/.test(value)) {
    const populated = continuation.filter((line) => line.trim() !== '')
    const indent = populated.length ? Math.min(...populated.map((line) => line.match(/^[ \t]*/)?.[0].length ?? 0)) : 0
    const lines = continuation.map((line) => line.slice(indent))
    if (value.startsWith('|')) return lines.join('\n').replace(/\n+$/, '')

    const paragraphs: string[] = []
    let paragraph: string[] = []
    for (const line of lines) {
      if (line.trim() === '') {
        if (paragraph.length) paragraphs.push(paragraph.join(' '))
        paragraph = []
      } else {
        paragraph.push(line.trim())
      }
    }
    if (paragraph.length) paragraphs.push(paragraph.join(' '))
    return paragraphs.join('\n\n')
  }
  if (value.startsWith('"') && value.endsWith('"')) {
    try {
      return JSON.parse(value)
    } catch {
      return value.slice(1, -1)
    }
  }
  if (value.startsWith("'") && value.endsWith("'")) {
    return value.slice(1, -1).replace(/''/g, "'")
  }
  if (value === '~' || value === 'null') return ''
  return value.replace(/\s+#.*$/, '').trim()
}

/** Read the first present top-level YAML field, falling back to `defaultKey`. */
export function readFrontmatterField(
  content: string,
  keys: readonly string[],
  defaultKey = keys[0] ?? 'name',
): FrontmatterField {
  const { yaml } = splitFrontmatter(content)
  const lines = yaml ? yaml.split(/\r?\n/) : []
  for (const key of keys) {
    const range = fieldRange(lines, key)
    if (!range) continue
    const [start, end] = range
    const raw = lines[start].slice(lines[start].indexOf(':') + 1)
    return {
      key,
      raw: lines.slice(start, end).join(/\r\n/.test(yaml) ? '\r\n' : '\n'),
      value: decodeScalar(raw, lines.slice(start + 1, end)),
      present: true,
    }
  }
  return { key: defaultKey, value: '', present: false }
}

/** Replace one managed field while leaving every other YAML key untouched. */
export function setFrontmatterField(content: string, key: string, value: string, bare = false): string {
  const parts = splitFrontmatter(content)
  const lines = parts.yaml ? parts.yaml.split(/\r?\n/) : []
  const range = fieldRange(lines, key)
  const encoded = bare && /^[a-z0-9_-]*$/.test(value) ? value : JSON.stringify(value)
  const replacement = `${key}: ${encoded}`
  if (range) lines.splice(range[0], range[1] - range[0], replacement)
  else lines.push(replacement)

  return joinFrontmatter({
    ...parts,
    hasFrontmatter: true,
    yaml: lines.join(parts.eol),
  })
}

/** Hide managed form fields from the markdown editor, retaining advanced YAML. */
export function withoutFrontmatterFields(content: string, keys: readonly string[]): string {
  const parts = splitFrontmatter(content)
  if (!parts.hasFrontmatter) return content

  const lines = parts.yaml ? parts.yaml.split(/\r?\n/) : []
  for (const key of new Set(keys)) {
    const range = fieldRange(lines, key)
    if (range) lines.splice(range[0], range[1] - range[0])
  }
  const yaml = lines.join(parts.eol).trim()
  return yaml ? joinFrontmatter({ ...parts, yaml }) : parts.body
}

/** Put hidden managed fields back before sending the full document to save. */
export function restoreFrontmatterFields(source: string, fields: readonly FrontmatterField[]): string {
  return fields.reduce((content, field) => {
    if (!field.present) return content
    if (!field.raw) {
      return setFrontmatterField(content, field.key, field.value, field.bare)
    }

    const parts = splitFrontmatter(content)
    const lines = parts.yaml ? parts.yaml.split(/\r?\n/) : []
    const range = fieldRange(lines, field.key)
    const replacement = field.raw.split(/\r?\n/)
    if (range) lines.splice(range[0], range[1] - range[0], ...replacement)
    else lines.push(...replacement)
    return joinFrontmatter({
      ...parts,
      hasFrontmatter: true,
      yaml: lines.join(parts.eol),
    })
  }, source)
}

export function frontmatterBody(content: string): string {
  return splitFrontmatter(content).body
}
