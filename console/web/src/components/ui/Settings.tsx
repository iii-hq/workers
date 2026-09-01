import uiClasses from '@iii-dev/console-ui/ui-classes'
import * as React from 'react'
import { cn } from '@/lib/utils'

function hasContent(value: React.ReactNode): boolean {
  return React.Children.toArray(value).length > 0
}

export interface SettingsSectionProps
  extends Omit<React.HTMLAttributes<HTMLElement>, 'title'> {
  title?: React.ReactNode
  description?: React.ReactNode
  action?: React.ReactNode
}

/** A titled settings group with an optional section-level action. */
export const SettingsSection = React.forwardRef<
  HTMLElement,
  SettingsSectionProps
>(
  (
    {
      className,
      title,
      description,
      action,
      children,
      'aria-labelledby': ariaLabelledBy,
      'aria-describedby': ariaDescribedBy,
      ...props
    },
    ref,
  ) => {
    const generatedId = React.useId()
    const titleId = `${generatedId}-title`
    const descriptionId = `${generatedId}-description`
    const hasTitle = hasContent(title)
    const hasDescription = hasContent(description)
    const hasAction = hasContent(action)
    const hasHeader = hasTitle || hasDescription || hasAction

    return (
      <section
        ref={ref}
        aria-labelledby={ariaLabelledBy ?? (hasTitle ? titleId : undefined)}
        aria-describedby={
          ariaDescribedBy ?? (hasDescription ? descriptionId : undefined)
        }
        className={cn(uiClasses.settingsSection, className)}
        {...props}
      >
        {hasHeader ? (
          <div className={uiClasses.settingsSectionHeader}>
            <div className={uiClasses.settingsSectionCopy}>
              {hasTitle ? (
                <h2 id={titleId} className={uiClasses.settingsSectionTitle}>
                  {title}
                </h2>
              ) : null}
              {hasDescription ? (
                <p
                  id={descriptionId}
                  className={uiClasses.settingsSectionDescription}
                >
                  {description}
                </p>
              ) : null}
            </div>
            {hasAction ? (
              <div className={uiClasses.settingsSectionAction}>{action}</div>
            ) : null}
          </div>
        ) : null}
        {children}
      </section>
    )
  },
)
SettingsSection.displayName = 'SettingsSection'

export const SettingsList = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, role = 'list', ...props }, ref) => (
  <div
    ref={ref}
    role={role}
    className={cn(uiClasses.settingsList, className)}
    {...props}
  />
))
SettingsList.displayName = 'SettingsList'

export type SettingsRowLayout = 'auto' | 'inline' | 'stacked'

export interface SettingsRowProps
  extends Omit<React.HTMLAttributes<HTMLDivElement>, 'children'> {
  label: React.ReactNode
  description?: React.ReactNode
  meta?: React.ReactNode
  /** Primary interactive or value control on the trailing edge. */
  control?: React.ReactNode
  /** Optional secondary action, composed after `control`. */
  action?: React.ReactNode
  /** `auto` stacks the trailing slot in narrow containers. */
  layout?: SettingsRowLayout
}

/** A key/value settings row with a responsive trailing control/action slot. */
export const SettingsRow = React.forwardRef<HTMLDivElement, SettingsRowProps>(
  (
    {
      className,
      label,
      description,
      meta,
      control,
      action,
      layout = 'auto',
      role = 'listitem',
      ...props
    },
    ref,
  ) => {
    const hasControl = hasContent(control)
    const hasAction = hasContent(action)
    const hasTrailing = hasControl || hasAction

    return (
      <div
        ref={ref}
        role={role}
        data-layout={layout}
        data-has-trailing={hasTrailing || undefined}
        className={cn(uiClasses.settingsRow, className)}
        {...props}
      >
        <div className={uiClasses.settingsRowInner}>
          <div className={uiClasses.settingsRowCopy}>
            <div className={uiClasses.settingsRowLabel}>{label}</div>
            {hasContent(description) ? (
              <div className={uiClasses.settingsRowDescription}>
                {description}
              </div>
            ) : null}
            {hasContent(meta) ? (
              <div className={uiClasses.settingsRowMeta}>{meta}</div>
            ) : null}
          </div>
          {hasTrailing ? (
            <div className={uiClasses.settingsRowControl}>
              {hasControl ? control : null}
              {hasAction ? action : null}
            </div>
          ) : null}
        </div>
      </div>
    )
  },
)
SettingsRow.displayName = 'SettingsRow'

export interface SettingsFieldControlProps {
  id: string
  name?: string
  'data-field'?: string
  'aria-invalid'?: true
  'aria-describedby'?: string
}

export type SettingsFieldControlSize = 'fit' | 'compact' | 'default' | 'full'

export interface SettingsFieldProps
  extends Omit<SettingsRowProps, 'label' | 'description' | 'meta' | 'control'> {
  id?: string
  name?: string
  /** Stable configuration path used by global-settings deep links. */
  field?: string
  label: React.ReactNode
  description?: React.ReactNode
  meta?: React.ReactNode
  error?: React.ReactNode
  controlSize?: SettingsFieldControlSize
  renderControl: (props: SettingsFieldControlProps) => React.ReactNode
}

const settingsFieldWidths: Record<SettingsFieldControlSize, string> = {
  fit: 'w-full sm:w-auto',
  compact: 'w-full sm:w-32',
  default: 'w-full sm:w-80',
  full: 'w-full',
}

/**
 * A labelled SettingsRow that owns IDs, validation associations, deep-link
 * metadata, and standard control widths. The render prop works for Input,
 * Select, Switch, and domain-specific controls without duplicating ARIA glue.
 */
export const SettingsField = React.forwardRef<
  HTMLDivElement,
  SettingsFieldProps
>(
  (
    {
      id,
      name,
      field,
      label,
      description,
      meta,
      error,
      controlSize = 'default',
      renderControl,
      ...props
    },
    ref,
  ) => {
    const generatedId = React.useId()
    const controlId = id ?? `${generatedId}-control`
    const hasDescription = hasContent(description)
    const hasMeta = hasContent(meta)
    const hasError = hasContent(error)
    const descriptionId = hasDescription
      ? `${generatedId}-description`
      : undefined
    const errorId = hasError ? `${generatedId}-error` : undefined
    const describedBy =
      [descriptionId, errorId].filter(Boolean).join(' ') || undefined

    return (
      <SettingsRow
        ref={ref}
        label={<label htmlFor={controlId}>{label}</label>}
        description={
          hasDescription ? (
            <span id={descriptionId}>{description}</span>
          ) : undefined
        }
        meta={
          hasMeta || hasError ? (
            <div className="flex flex-col items-start gap-0.5">
              {hasMeta ? meta : null}
              {hasError ? (
                <span id={errorId} className="text-alert" role="alert">
                  {error}
                </span>
              ) : null}
            </div>
          ) : undefined
        }
        control={
          <div
            data-settings-field-control
            className={cn(
              'min-w-0 [&>*]:max-w-full [&>*]:w-full',
              settingsFieldWidths[controlSize],
            )}
          >
            {renderControl({
              id: controlId,
              name: name ?? field,
              'data-field': field,
              'aria-invalid': hasError ? true : undefined,
              'aria-describedby': describedBy,
            })}
          </div>
        }
        {...props}
      />
    )
  },
)
SettingsField.displayName = 'SettingsField'
