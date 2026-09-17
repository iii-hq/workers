export function leafName(path: string) {
  const clean = path.endsWith('/') ? path.slice(0, -1) : path
  return clean.slice(clean.lastIndexOf('/') + 1) || clean
}

export function parentPrefix(prefix: string) {
  const clean = prefix.endsWith('/') ? prefix.slice(0, -1) : prefix
  const slash = clean.lastIndexOf('/')
  return slash < 0 ? '' : clean.slice(0, slash + 1)
}
