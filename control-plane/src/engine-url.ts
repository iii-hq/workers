const wildcardHost = /^[^:]*:[/\\]*(?:[^/?#\\]*@)?(0\.0\.0\.0)(?=(?::[^@/?#\\]*)?(?:[/?#\\]|$))/d

export function resolveEngineUrl(url: string | undefined): string | undefined {
  if (!url) return url

  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    return url
  }
  if ((parsed.protocol !== 'ws:' && parsed.protocol !== 'wss:') || parsed.hostname !== '0.0.0.0') {
    return url
  }

  // Match the literal spelling after WHATWG preprocessing, retaining original offsets.
  const characters = [...url.matchAll(/[^\t\n\r]/g)]
  while (characters.length && characters[characters.length - 1][0].charCodeAt(0) <= 0x20) {
    characters.pop()
  }
  const host = wildcardHost.exec(characters.map(([character]) => character).join(''))?.indices?.[1]
  if (!host) return url

  const [start, end] = host
  const hostStart = characters[start].index
  const hostEnd = characters[end - 1].index + 1
  return `${url.slice(0, hostStart)}127.0.0.1${url.slice(hostEnd)}`
}
