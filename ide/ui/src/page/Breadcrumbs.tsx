/* The path of the file in the main pane as clickable segments — VS Code's
   breadcrumbs. A folder segment reveals that folder in the explorer; the
   last segment is the file itself. */

import { ChevronRight } from 'lucide-react'
import { FileTypeIcon } from './file-type-icon'
import { breadcrumbSegments } from './paths'

export function Breadcrumbs({
  path,
  rootLabel,
  onSelectDir,
}: {
  path: string
  rootLabel: string
  onSelectDir: (dir: string) => void
}) {
  const segments = breadcrumbSegments(path)
  return (
    <nav className="shui-breadcrumbs" aria-label="File path">
      <button type="button" className="crumb root" onClick={() => onSelectDir('')} title={rootLabel}>
        {rootLabel}
      </button>
      {segments.map((segment, index) => {
        const last = index === segments.length - 1
        return (
          <span key={segment.path} className="crumb-group">
            <ChevronRight aria-hidden className="sep" />
            {last ? (
              <span className="crumb file" title={path}>
                <FileTypeIcon path={path} className="crumb-icon" />
                {segment.name}
              </span>
            ) : (
              <button type="button" className="crumb" onClick={() => onSelectDir(segment.path)} title={segment.path}>
                {segment.name}
              </button>
            )}
          </span>
        )
      })}
    </nav>
  )
}
