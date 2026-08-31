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
