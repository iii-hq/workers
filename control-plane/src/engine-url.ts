const wildcardAuthority =
  /^(?<prefix>\s*wss?:[\\/]*(?:[^/?#\\]*@)?)0\.0\.0\.0(?=(?::[^@/?#\\]*)?(?:[/?#\\]|\s*$))/i

export function resolveEngineUrl(url: string | undefined): string | undefined {
  if (!url || !URL.canParse(url)) return url

  // Replace only the literal host so URL serialization cannot change the rest.
  return url.replace(wildcardAuthority, '$<prefix>127.0.0.1')
}
