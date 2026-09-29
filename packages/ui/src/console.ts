export type ConsoleNavItem = {
  to: string
  label: string
  icon: string
  locked?: boolean
  lockedLabel?: string
  disabled?: boolean
  disabledLabel?: string
}
export type ConsoleNavSection = { label?: string; items: ConsoleNavItem[] }

export type ConsoleQuickLink = {
  label: string
  icon: string
  to?: string
  href?: string
  keepVisibleOnMobile?: boolean
}
