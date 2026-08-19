import { FileDiff, type Host } from '@iii-dev/console-ui'
import { Eye } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { errorMessage } from '../lib/format'

interface ChangeDiffResponse {
  path: string
  old_contents?: string
  new_contents?: string
  is_binary: boolean
}

type DiffState =
  | { phase: 'loading' }
  | { phase: 'error'; message: string }
  | { phase: 'ready'; value: ChangeDiffResponse }

interface ChangeDiffPaneProps {
  host: Host
  changeId: string
  path: string
  canViewFile: boolean
  onViewFile: (path: string) => void
}

export function ChangeDiffPane({ host, changeId, path, canViewFile, onViewFile }: ChangeDiffPaneProps) {
  const [state, setState] = useState<DiffState>({ phase: 'loading' })
  const seqRef = useRef(0)

  useEffect(() => {
    const seq = ++seqRef.current
    let active = true
    setState({ phase: 'loading' })
    host.iii
      .trigger<ChangeDiffResponse>('coder::change-diff', {
        change_id: changeId,
      })
      .then((value) => {
        if (active && seqRef.current === seq) setState({ phase: 'ready', value })
      })
      .catch((error: unknown) => {
        if (active && seqRef.current === seq) {
          setState({ phase: 'error', message: errorMessage(error) })
        }
      })
    return () => {
      active = false
    }
  }, [changeId, host])

  const resolvedPath = state.phase === 'ready' ? state.value.path : path

  return (
    <div className="shui-main-pane">
      <div className="shui-editor-head">
        <span className="path" title={resolvedPath}>
          {resolvedPath}
        </span>
        <span className="meta">exact change</span>
        <span className="spacer" />
        {canViewFile ? (
          <button type="button" className="shui-view-file-btn" onClick={() => onViewFile(resolvedPath)}>
            <Eye aria-hidden />
            View file
          </button>
        ) : null}
      </div>
      <div className="shui-editor-body">
        {state.phase === 'loading' ? (
          <div className="shui-side-note">loading exact diff…</div>
        ) : state.phase === 'error' ? (
          <div className="shui-side-note warn">{state.message}</div>
        ) : state.value.is_binary ? (
          <div className="shui-side-note warn">binary file — no text diff</div>
        ) : (
          <FileDiff
            oldFile={{
              name: state.value.path,
              contents: state.value.old_contents ?? '',
            }}
            newFile={{
              name: state.value.path,
              contents: state.value.new_contents ?? '',
            }}
          />
        )}
      </div>
    </div>
  )
}
