import { Button } from '@iii-dev/console-ui'
import { useEffect, useId, useRef } from 'react'
import type { InvestigationProfile } from '../api'

/** Inline on purpose: Select's public API cannot bound its portalled options.
 * Keeping this small list inside the dialog also retains its focus/scope boundary. */
export function ProfilePicker({ profiles, value, disabled, loading, open, onOpenChange, onChange }: {
  profiles: InvestigationProfile[]
  value: string
  disabled: boolean
  loading: boolean
  open: boolean
  onOpenChange: (open: boolean) => void
  onChange: (value: string) => void
}) {
  const wasOpen = useRef(false)
  const trigger = useRef<HTMLButtonElement>(null)
  const list = useRef<HTMLDivElement>(null)
  const id = useId()
  const chosen = profiles.find(profile => profile.id === value)
  const close = () => { onOpenChange(false); trigger.current?.focus() }
  useEffect(() => {
    if (disabled) onOpenChange(false)
  }, [disabled])
  useEffect(() => {
    if (open) {
      const options = list.current?.querySelectorAll<HTMLButtonElement>('[role="option"]:not(:disabled)')
      const selected = Array.from(options ?? []).find(option => option.getAttribute('aria-selected') === 'true')
      ;(selected ?? options?.[0])?.focus()
    }
    if (!open && wasOpen.current && !disabled) trigger.current?.focus()
    wasOpen.current = open
  }, [open, disabled])
  return <div data-iii-ui="sentinel" className="sentinel-ui-profile-picker">
    <Button ref={trigger} variant="ghost" className="sentinel-ui-profile-trigger" role="combobox" aria-label="Investigation profile" aria-haspopup="listbox" aria-expanded={open} aria-controls={id} disabled={disabled} onClick={() => onOpenChange(!open)} onKeyDown={event => {
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); onOpenChange(true) }
    }}>{chosen ? `${chosen.name} · ${chosen.id}` : loading ? 'Loading installed profiles…' : 'Choose an installed profile'}</Button>
    {open ? <div ref={list} id={id} role="listbox" aria-label="Installed investigation profiles" className="sentinel-ui-profile-options" onKeyDown={event => {
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); close(); return }
      if (event.key === 'Tab') { onOpenChange(false); return }
      const options = Array.from(list.current?.querySelectorAll<HTMLButtonElement>('[role="option"]:not(:disabled)') ?? [])
      const index = options.indexOf(event.target as HTMLButtonElement)
      let next = index
      if (event.key === 'ArrowDown') next = (index + 1) % options.length
      else if (event.key === 'ArrowUp') next = (index - 1 + options.length) % options.length
      else if (event.key === 'Home') next = 0
      else if (event.key === 'End') next = options.length - 1
      else return
      event.preventDefault()
      options[next]?.focus()
    }}>{profiles.map(profile => <button key={profile.id} type="button" role="option" aria-selected={value === profile.id} disabled={!!profile.inheritance_error} tabIndex={-1} className="sentinel-ui-profile-option" onClick={() => { onChange(profile.id); close() }}>
      <span className="sentinel-ui-profile-name">{profile.name} · {profile.id}</span>
      <span className="sentinel-ui-profile-description">{profile.description}</span>
      {profile.inheritance_error ? <span className="sentinel-ui-profile-description">Unavailable: {profile.inheritance_error}</span> : null}
    </button>)}</div> : null}
  </div>
}
