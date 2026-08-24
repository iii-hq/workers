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

export function registerShellPalette(host: Host): void {
  host.palette?.registerSource({
    id: 'files',
    title: 'Files',
    kind: 'file',
    prefix: '#',
    minQuery: 3,
    async search(query, { workingDir, signal }) {
      const base = workingDir ?? (await coderInfo(host)).primary_root
      if (signal.aborted) return []
      const out = await coderSearch(host, {
        query,
        regex: false,
        ignoreCase: true,
        path: base,
        searchContent: false,
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
                pageId: 'shell',
                context: { type: 'file', path: match.path },
              }),
          }
        })
    },
  })

  host.commands?.register('shell', [
    {
      id: 'open-file',
      title: 'Open file…',
      detail: 'Find a file by name and open it in the shell',
      keywords: ['quick open', 'file', 'go to file', 'path'],
      run: () => host.palette?.open({ query: '#' }),
    },
    {
      id: 'open',
      title: 'Open the shell',
      detail: 'Files, search, changes and a terminal for the working directory',
      keywords: ['terminal', 'explorer', 'files', 'ide'],
      run: () => host.panels?.open({ pageId: 'shell', context: {} }),
    },
  ])
}
