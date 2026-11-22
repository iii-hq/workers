import { errorMessage } from '@iii-dev/console-ui/format'
import { useCallback, useEffect, useId, useRef, useState } from 'react'
import type { IiiClient } from '@/lib/iii-client'
import type { TowerData } from './data'
import { ALL_PROJECTS } from './model'
import {
  acceptSummary,
  SUMMARY_INTERVAL,
  SUMMARY_SCHEMA,
  type TowerSummary,
  WRITER_READS,
} from './summary'

export function useTowerSummary(
  iii: IiiClient,
  scope: string,
  data: TowerData,
) {
  const [snapshot, setSnapshot] = useState<{
    scope: string
    result: TowerSummary | null
    loading: boolean
    error: string | null
  }>({ scope, result: null, loading: true, error: null })
  const [clock, setClock] = useState(Date.now)
  const input = useRef(data)
  input.current = data
  const controller = useRef<{
    scope: string
    run: (force: boolean, manual?: boolean) => Promise<void>
  } | null>(null)
  const instance = useId().replace(/[^a-z0-9]/gi, '')
  const ready =
    !data.loading.board && !data.loading.sessions && !data.loading.prs

  useEffect(() => {
    const sessionId = `tower-${scope}`
    const handlerId = `iii::console::tower::summary-${instance}`
    let active = true
    let busy = false
    let result: TowerSummary | null = null
    let opened = false
    let pendingRefresh: string | false | null = null
    let queuedRefresh = false
    let observerError: string | null = null
    let debounce: ReturnType<typeof setTimeout> | undefined
    const publish = (loading: boolean, error: string | null = null) => {
      if (active) {
        setClock(Date.now())
        setSnapshot({ scope, result, loading, error })
      }
    }
    const accept = (value: unknown) => {
      const next = acceptSummary(value, result)
      const rejected = next === result
      result = next
      return rejected ? 'The tower writer returned an invalid summary.' : null
    }
    const run = async (force: boolean, manual = false) => {
      if (manual) pendingRefresh = `refresh:${crypto.randomUUID()}`
      else if (force && pendingRefresh === null) pendingRefresh = false
      if (!active) return
      if (busy) {
        if (pendingRefresh !== null) queuedRefresh = true
        return
      }
      if (opened && pendingRefresh === null) return
      busy = true
      const refreshRequest = pendingRefresh
      pendingRefresh = null
      const work = async () => {
        try {
          if (!active) return
          if (observerError) {
            publish(false, observerError)
            return
          }
          const status = await iii.call<{
            turn_id?: string | null
            result?: unknown
            status?: string
            result_error?: string | null
          } | null>('harness::status', { session_id: sessionId, verbose: true })
          if (!active) return
          const error =
            status?.result_error ??
            (status?.result == null
              ? null
              : status.status === 'completed'
                ? accept(status.result)
                : 'The tower writer did not complete its summary.')
          publish(false, error)
          const current = input.current
          if (
            current.loading.board ||
            current.loading.sessions ||
            current.loading.prs
          ) {
            if (pendingRefresh === null) pendingRefresh = refreshRequest
            return
          }
          opened = true
          if (
            refreshRequest === null &&
            result &&
            Date.now() - Date.parse(result.checked_at) <= SUMMARY_INTERVAL
          )
            return
          publish(true, error)
          const ensured = await iii.call<{
            meta: {
              kind?: string
              metadata?: { origin?: string; agent_profile?: { id?: string } }
            }
          }>('session::ensure', {
            session_id: sessionId,
            title: sessionId,
            kind: 'automation',
            metadata: { origin: 'tower' },
          })
          if (!active) return
          if (
            ensured.meta.kind !== 'automation' ||
            ensured.meta.metadata?.origin !== 'tower' ||
            (status?.turn_id &&
              ensured.meta.metadata?.agent_profile?.id !== 'tower-writer')
          )
            throw new Error(
              'The tower session belongs to another agent or origin.',
            )
          await iii.call('approval::set-mode', {
            session_id: sessionId,
            mode: 'auto',
          })
          for (const functionId of WRITER_READS) {
            if (!active) return
            await iii.call('approval::add-always-allow', {
              session_id: sessionId,
              function_id: functionId,
            })
          }
          if (!active) return
          const refreshAt = new Date().toISOString()
          const slot = Math.floor(Date.now() / SUMMARY_INTERVAL)
          const refreshSuffix =
            typeof refreshRequest === 'string' ? refreshRequest : null
          const sent = await iii.call<{ deduplicated?: boolean }>(
            'harness::send',
            {
              session_id: sessionId,
              idempotency_key: `tower:${scope}:${slot}${refreshSuffix ? `:${refreshSuffix}` : ''}`,
              message: JSON.stringify({
                checked_at: result?.checked_at ?? null,
                refresh_at: refreshAt,
                project_id: scope === ALL_PROJECTS ? null : scope,
                repos: current.repositories,
                session_ids: current.runs.map((run) => run.session.session_id),
              }),
              options: {
                ...(!status?.turn_id ? { agent: 'tower-writer' } : {}),
                functions: { allow: WRITER_READS },
                output: { type: 'json', schema: SUMMARY_SCHEMA },
              },
            },
          )
          if (
            sent.deduplicated &&
            status?.status &&
            ['completed', 'failed', 'cancelled'].includes(status.status)
          )
            publish(false, error)
        } catch (error) {
          publish(false, errorMessage(error))
        } finally {
          busy = false
          if (active && queuedRefresh) {
            queuedRefresh = false
            void run(false)
          }
        }
      }
      // Serialize first use across tabs before the harness freezes the profile.
      const locks = navigator.locks
      if (locks) await locks.request(`iii-tower-writer:${sessionId}`, work)
      else await work()
    }
    controller.current = { scope, run }
    publish(true)
    const offs: Array<() => void> = []
    try {
      offs.push(
        iii.on<{
          session_id?: string
          status?: string
          result?: unknown
          result_error?: string | null
        }>(handlerId, (event) => {
          if (event.session_id !== sessionId) return
          const error =
            event.result_error ??
            (event.status === 'completed'
              ? accept(event.result)
              : 'The tower writer did not complete its summary.')
          clearTimeout(debounce)
          debounce = setTimeout(() => {
            if (active) publish(false, error)
          }, 2_000)
        }),
      )
      offs.push(
        iii.observe(
          {
            function_id: 'harness::on-turn-completed',
            filters: { 'input.session_id': { $eq: sessionId } },
          },
          `${handlerId}::${iii.browserId}`,
        ),
      )
    } catch (error) {
      observerError = errorMessage(error)
      publish(false, observerError)
    }
    void run(false)
    const interval = window.setInterval(() => void run(true), SUMMARY_INTERVAL)
    const minute = window.setInterval(() => setClock(Date.now()), 60_000)
    return () => {
      active = false
      clearTimeout(debounce)
      window.clearInterval(interval)
      window.clearInterval(minute)
      for (const off of offs) off()
    }
  }, [iii, instance, scope])

  useEffect(() => {
    if (ready && controller.current?.scope === scope)
      void controller.current.run(false)
  }, [ready, scope])
  const refresh = useCallback(() => {
    void controller.current?.run(true, true)
  }, [])
  return {
    result: snapshot.scope === scope ? snapshot.result : null,
    loading: snapshot.scope !== scope || snapshot.loading,
    error: snapshot.scope === scope ? snapshot.error : null,
    refresh,
    now: clock,
  }
}
