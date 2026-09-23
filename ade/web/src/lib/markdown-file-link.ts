import { type FileMentionRef, normalizeLineRange } from './file-mention-token'

function fileShaped(path: string): boolean {
  const basename = path.slice(path.lastIndexOf('/') + 1)
  return /\.[\w-]+$/.test(basename) || /^(?:README|LICENSE|COPYING|NOTICE|Dockerfile|Containerfile|Makefile|Justfile|Procfile)$/i.test(basename)
}

/** Parse syntax before decoding so %23, %3F and %3A remain filename bytes. */
export function parseMarkdownFileLink(href: string): FileMentionRef | null {
  if (!href || href.trim() !== href || href.includes('?')) return null
  let destination = href
  let explicit = false
  if (/^file:\/\//i.test(destination)) {
    const local = destination.match(/^file:\/\/(?:localhost)?(\/.*)$/i)
    if (!local) return null
    destination = local[1]
    explicit = true
  }
  if (/^[a-z][a-z\d+.-]*:\/\//i.test(destination)) return null
  const hash = destination.indexOf('#')
  const fragment = hash < 0 ? '' : destination.slice(hash + 1)
  let encoded = hash < 0 ? destination : destination.slice(0, hash)
  const suffix = encoded.match(/:(\d+)(?:(?:[-–](\d+))|:(\d+))?$/)
  if (suffix) encoded = encoded.slice(0, suffix.index)
  // Reject schemes and unsupported coordinate formats before decoding.
  if (encoded.includes(':')) return null
  const position = fragment ? fragment.match(/^L(\d+)(?:[-–]L?(\d+))?$/) : suffix
  if ((hash >= 0 && !fragment) || (fragment && !position) || (suffix && fragment)) return null
  let path: string
  try {
    path = decodeURIComponent(encoded)
  } catch {
    return null
  }
  if (
    !path || path.includes('\\') || path.startsWith('//') || path.endsWith('/') ||
    /^[a-z][a-z\d+.-]*:/i.test(path) ||
    Array.from(path).some((char) => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127)
  ) return null
  const basename = path.slice(path.lastIndexOf('/') + 1)
  if (!basename || basename === '.' || basename === '..') return null
  if (!explicit && !(position && path.includes('/')) && !fileShaped(path)) return null
  if (!position) return { path }
  const from = Number(position[1])
  const to = Number(position[2] ?? position[1])
  const column = suffix?.[3] ? Number(suffix[3]) : undefined
  if (
    !Number.isSafeInteger(from) || !Number.isSafeInteger(to) || from < 1 || to < 1 ||
    (column !== undefined && (!Number.isSafeInteger(column) || column < 1))
  ) return null
  return { path, range: normalizeLineRange(from, to), ...(column ? { column } : {}) }
}

/** File-looking invalid links must not silently become web navigation. */
export function markdownFileLinkError(href: string): string | null {
  if (parseMarkdownFileLink(href)) return null
  if (/^(?:file:|[a-z]:[\\/])/i.test(href)) {
    return 'Unsupported file URL or Windows path. Use a local Unix path; remote file hosts are not supported.'
  }
  if (/^[a-z][a-z\d+.-]*:/i.test(href) && !fileShaped(href.split(':')[0])) return null
  if (href.startsWith('//') || href.startsWith('#')) return null
  if (fileShaped(href.split(/[?#:]/)[0]) && (/[#:%]/.test(href) || href.includes('\\'))) {
    return 'Invalid file reference. Use path#L10, path#L10-L20 or path:10:5; line and column numbers must be positive integers.'
  }
  return null
}
