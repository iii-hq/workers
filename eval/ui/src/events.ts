import { useEffect } from 'react'
import type { Host } from '@iii-dev/console-ui'
import type { EvalStatus } from './types'

const COMPLETED_FN = 'iii::eval-ui::completed'

export interface EvalCompletedEvent {
  evaluation_id: string
  status: EvalStatus
  eligible?: boolean
  timestamp: number
}

export function useEvalCompleted(
  host: Host,
  onCompleted: (event: EvalCompletedEvent) => void,
) {
  useEffect(() => {
    const offHandler = host.iii.on<EvalCompletedEvent>(
      COMPLETED_FN,
      onCompleted,
    )
    const offTrigger = host.iii.registerTrigger({
      type: 'eval::completed',
      function_id: `${COMPLETED_FN}::${host.iii.browserId}`,
      config: {},
    })
    return () => {
      offTrigger()
      offHandler()
    }
  }, [host, onCompleted])
}
