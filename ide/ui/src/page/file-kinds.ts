/* File-name driven facts every pane agrees on: which Monaco language a
   path gets and which paths are raster images. React-free so loaders and
   tests can import it without the editor. */

/** File-extension → Monaco language id (unknown ids render plain). */
export function monacoLangFromPath(path: string): string {
  const lower = path.toLowerCase()
  if (lower.endsWith('dockerfile')) return 'dockerfile'
  if (lower.endsWith('makefile')) return 'shell'
  const ext = lower.match(/\.([a-z0-9]+)$/)?.[1]
  switch (ext) {
    case 'ts':
    case 'tsx':
    case 'mts':
    case 'cts':
      return 'typescript'
    case 'js':
    case 'jsx':
    case 'mjs':
    case 'cjs':
      return 'javascript'
    case 'json':
    case 'jsonc':
      return 'json'
    case 'yml':
    case 'yaml':
      return 'yaml'
    case 'md':
    case 'mdx':
      return 'markdown'
    case 'rs':
      return 'rust'
    case 'go':
      return 'go'
    case 'py':
    case 'pyi':
      return 'python'
    case 'rb':
      return 'ruby'
    case 'sh':
    case 'bash':
    case 'zsh':
      return 'shell'
    case 'html':
    case 'htm':
      return 'html'
    case 'css':
      return 'css'
    case 'scss':
      return 'scss'
    case 'less':
      return 'less'
    case 'sql':
      return 'sql'
    case 'xml':
    case 'svg':
      return 'xml'
    case 'toml':
    case 'ini':
      return 'ini'
    case 'java':
      return 'java'
    case 'kt':
    case 'kts':
      return 'kotlin'
    case 'swift':
      return 'swift'
    case 'c':
    case 'h':
      return 'c'
    case 'cpp':
    case 'cc':
    case 'hpp':
      return 'cpp'
    case 'cs':
      return 'csharp'
    case 'php':
      return 'php'
    case 'graphql':
    case 'gql':
      return 'graphql'
    default:
      return 'plaintext'
  }
}

/** Extension → MIME for the image preview (SVG stays in the text editor
    — it's editable markup first). */
export function imageMimeFromPath(path: string): string | null {
  const ext = path.toLowerCase().match(/\.([a-z0-9]+)$/)?.[1]
  switch (ext) {
    case 'png':
      return 'image/png'
    case 'jpg':
    case 'jpeg':
      return 'image/jpeg'
    case 'gif':
      return 'image/gif'
    case 'webp':
      return 'image/webp'
    case 'bmp':
      return 'image/bmp'
    case 'ico':
      return 'image/x-icon'
    case 'avif':
      return 'image/avif'
    default:
      return null
  }
}
