/**
 * Stable CSS recipes for native and injected Console UI. These names are
 * intentionally independent from Tailwind so worker pages can compose the
 * shared visual language without copying utility lists into every bundle.
 * State is expressed with data attributes such as `data-selected` and
 * `data-tone`.
 */
export const uiClasses = Object.freeze({
  list: 'iii-ui-list',
  listGroup: 'iii-ui-list-group',
  listGroupLabel: 'iii-ui-list-group__label',
  listItem: 'iii-ui-list-item',
  listItemIcon: 'iii-ui-list-item__icon',
  listItemContent: 'iii-ui-list-item__content',
  listItemTitle: 'iii-ui-list-item__title',
  listItemDescription: 'iii-ui-list-item__description',
  listItemMeta: 'iii-ui-list-item__meta',
  card: 'iii-ui-card',
  cardHeader: 'iii-ui-card__header',
  cardBody: 'iii-ui-card__body',
  cardHighlight: 'iii-ui-card-highlight',
  collapsibleCard: 'iii-ui-collapsible-card',
  collapsibleCardTrigger: 'iii-ui-collapsible-card__trigger',
  collapsibleCardContent: 'iii-ui-collapsible-card__content',
  collapsibleCardContentInner: 'iii-ui-collapsible-card__content-inner',
  panel: 'iii-ui-panel',
  panelHeader: 'iii-ui-panel__header',
  panelBody: 'iii-ui-panel__body',
  chip: 'iii-ui-chip',
  icon: 'iii-ui-icon',
  tableViewport: 'iii-ui-table-viewport',
  tableFrame: 'iii-ui-table-frame',
  table: 'iii-ui-table',
  tableHeader: 'iii-ui-table__header',
  tableBody: 'iii-ui-table__body',
  tableFooter: 'iii-ui-table__footer',
  tableRow: 'iii-ui-table__row',
  tableHead: 'iii-ui-table__head',
  tableCell: 'iii-ui-table__cell',
  tableCaption: 'iii-ui-table__caption',
  tabsList: 'iii-ui-tabs-list',
  tab: 'iii-ui-tab',
  tabIcon: 'iii-ui-tab__icon',
  segmentedControl: 'iii-ui-segmented',
  segmentedItem: 'iii-ui-segmented__item',
  field: 'iii-ui-field',
  fieldLabel: 'iii-ui-field__label',
  fieldDescription: 'iii-ui-field__description',
  fieldError: 'iii-ui-field__error',
  motionControl: 'iii-ui-motion-control',
  motionPanel: 'iii-ui-motion-panel',
  motionOverlay: 'iii-ui-motion-overlay',
})

export const uiClassNames = Object.freeze(Object.values(uiClasses))

export default uiClasses
