/**
 * The shell in the command palette, before its page is even open.
 *
 * A files source answers `#` (and any query once it is a few characters)
 * with path matches from the coder worker under the chat's working
 * directory, each row opening that file in the shell page. Worker-level
 * commands open the page on a verb. Everything here is registered from
 * setup, so it exists only while the shell worker is connected; older
 * consoles without `host.palette` / `host.commands` simply get nothing.
 */

import type { Host } from '@iii-dev/console-ui'
import { coderInfo, coderSearch, relativeTo } from './coder'

const FILE_ROWS = 30

// The coder worker's root, read once per host rather than before every
// search; a failed read is forgotten so the next search tries again.
const primaryRoots = new WeakMap<Host, Promise<string>>()
function primaryRoot(host: Host): Promise<string> {
  let root = primaryRoots.get(host)
  if (root === undefined) {
    root = coderInfo(host).then((info) => info.primary_root)
    root.catch(() => primaryRoots.delete(host))
    primaryRoots.set(host, root)
  }
  return root
}

export function registerShellPalette(host: Host): void {
  host.palette?.registerSource({
    id: 'files',
    title: 'Files',
    kind: 'file',
    prefix: '#',
    minQuery: 3,
    async search(query, { workingDir, signal }) {
      const base = workingDir ?? (await primaryRoot(host))
      if (signal.aborted) return []
      // Quick-open ranking: the worker scores every path by fuzzy
      // subsequence match and returns the best first, skipping what
      // .gitignore hides.
      const out = await coderSearch(host, {
        query,
        regex: false,
        ignoreCase: true,
        path: base,
        searchContent: false,
        fuzzyPaths: true,
        respectGitignore: true,
        maxMatches: FILE_ROWS * 2,
      })
      if (signal.aborted) return []
      return out.path_matches
        .filter((match) => match.kind !== 'dir')
        .slice(0, FILE_ROWS)
        .map((match) => {
          const rel = relativeTo(base, match.path)
          const slash = rel.lastIndexOf('/')
          return {
            id: match.path,
            title: slash === -1 ? rel : rel.slice(slash + 1),
            detail: slash === -1 ? base : rel.slice(0, slash),
            keywords: [rel],
            run: () =>
              host.panels?.open({
                pageId: 'ide',
                context: { type: 'file', path: match.path },
              }),
          }
        })
    },
  })

  host.commands?.register('ide', [
    {
      id: 'open-file',
      title: 'Open file…',
      detail: 'Find a file by name and open it in the IDE',
      keywords: ['quick open', 'file', 'go to file', 'path'],
      run: () => host.palette?.open({ query: '#' }),
    },
    {
      id: 'new-worker',
      title: 'New worker…',
      detail: 'Create an iii worker from a template and add it to the stack',
      keywords: ['scaffold', 'template', 'worker', 'create', 'compose'],
      run: () => host.panels?.open({ pageId: 'ide', context: { type: 'new-worker' } }),
    },
    {
      id: 'open',
      title: 'Open the IDE',
      detail: 'Files, search, changes and a terminal for the working directory',
      keywords: ['terminal', 'explorer', 'files', 'ide'],
      run: () => host.panels?.open({ pageId: 'ide', context: {} }),
    },
  ])
}
