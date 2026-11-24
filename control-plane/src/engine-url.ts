const wildcardAuthority = /^(?<prefix>wss?:\/\/(?:[^/?#\\]*@)?)0\.0\.0\.0(?=(?::\d*)?(?:[/?#]|$))/i

export function resolveEngineUrl(url: string | undefined): string | undefined {
  if (!url || !URL.canParse(url)) return url

  // Replace only the literal host so URL serialization cannot change the rest.
  return url.replace(wildcardAuthority, '$<prefix>127.0.0.1')
}
