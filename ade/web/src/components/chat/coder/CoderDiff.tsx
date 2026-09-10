import type { FileContents } from '@pierre/diffs'
import { DEFAULT_THEMES } from '@pierre/diffs'
import { File, MultiFileDiff } from '@pierre/diffs/react'
import { Chip } from '@/components/chat/sandbox/terminal/Terminal'
import { useTheme } from '@/hooks/use-theme'

function usePierreDiffOptions() {
  const [theme] = useTheme()
  return {
    diffStyle: 'unified' as const,
    overflow: 'wrap' as const,
    theme: DEFAULT_THEMES,
    themeType: theme,
  }
}

/** Input-derived diff: empty old side for new files. */
export function CoderNewFilePreview({
  path,
  content,
}: {
  path: string
  content: string
}) {
  const options = usePierreDiffOptions()
  const oldFile: FileContents = { name: path, contents: '' }
  const newFile: FileContents = { name: path, contents: content }

  return (
    <div className="border-b border-rule-2 last:border-b-0">
      <MultiFileDiff oldFile={oldFile} newFile={newFile} options={options} />
    </div>
  )
}

/** Overwrite: previous content is not on the wire — show new body only. */
export function CoderOverwritePreview({
  path,
  content,
}: {
  path: string
  content: string
}) {
  const options = usePierreDiffOptions()
  const file: FileContents = { name: path, contents: content }

  return (
    <div className="border-b border-rule-2 last:border-b-0">
      <div className="bg-paper-2 border-b border-rule-2 px-3 py-2 flex flex-wrap items-center gap-1.5">
        <Chip label="path">{path}</Chip>
        <Chip label="overwrite" className="border-warn text-warn">
          true
        </Chip>
      </div>
      <File file={file} options={options} />
    </div>
  )
}
