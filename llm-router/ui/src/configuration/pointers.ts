export function pointer(...parts: Array<string | number>): string {
  return `/${parts.map((p) => String(p).replace(/~/g, '~0').replace(/\//g, '~1')).join('/')}`
}

export function errorAt(
  errors: ReadonlyMap<string, string> | undefined,
  ...parts: Array<string | number>
): string | null {
  if (!errors) return null
  return errors.get(pointer(...parts)) ?? null
}
