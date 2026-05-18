import type * as React from 'react'
import { cn } from '@/lib/utils'

interface SelectOption<T extends string> {
  value: T
  label: string
}

interface SelectGroup<T extends string> {
  label: string
  options: SelectOption<T>[]
}

interface SelectProps<T extends string>
  extends Omit<
    React.SelectHTMLAttributes<HTMLSelectElement>,
    'onChange' | 'value' | 'defaultValue'
  > {
  value: T
  options?: SelectOption<T>[]
  groups?: SelectGroup<T>[]
  onChange: (next: T) => void
}

export type { SelectGroup, SelectOption }

export function Select<T extends string>({
  value,
  options,
  groups,
  onChange,
  className,
  disabled,
  ...rest
}: SelectProps<T>) {
  return (
    <label
      className={cn(
        'inline-flex items-center gap-x-2 border border-rule bg-bg px-3 h-9 text-ink font-mono text-[13px] lowercase relative focus-within:border-ink transition-colors',
        disabled && 'opacity-40 pointer-events-none',
        className,
      )}
    >
      <select
        {...rest}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.currentTarget.value as T)}
        className="appearance-none bg-transparent pr-5 outline-none cursor-pointer lowercase"
      >
        {groups
          ? groups.map((g) => (
              <optgroup key={g.label} label={g.label}>
                {g.options.map((opt) => (
                  <option
                    key={opt.value}
                    value={opt.value}
                    className="text-ink bg-bg"
                  >
                    {opt.label}
                  </option>
                ))}
              </optgroup>
            ))
          : (options ?? []).map((opt) => (
              <option
                key={opt.value}
                value={opt.value}
                className="text-ink bg-bg"
              >
                {opt.label}
              </option>
            ))}
      </select>
      <span
        aria-hidden="true"
        className="absolute right-2 top-1/2 -translate-y-1/2 text-ink-faint pointer-events-none"
      >
        <svg
          width="8"
          height="6"
          viewBox="0 0 8 6"
          fill="none"
          stroke="currentColor"
          strokeWidth="1"
          aria-hidden="true"
        >
          <path d="M1 1L4 5L7 1" />
        </svg>
      </span>
    </label>
  )
}
