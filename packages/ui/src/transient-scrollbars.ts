const installedAttribute = 'data-aster-transient-scrollbars'
const hideDelayMs = 700
const trackScale = 0.95

function createIndicator(axis: 'vertical' | 'horizontal'): HTMLDivElement {
  const indicator = document.createElement('div')
  indicator.className = `a-transient-scrollbar is-${axis}`
  indicator.setAttribute('aria-hidden', 'true')
  document.body.append(indicator)
  return indicator
}

function scrollElement(target: EventTarget | null): Element | null {
  if (target === document) return document.scrollingElement
  return target instanceof Element ? target : null
}

export function installTransientScrollbars(): void {
  if (typeof document === 'undefined' || document.documentElement.hasAttribute(installedAttribute)) return
  document.documentElement.setAttribute(installedAttribute, '')

  let verticalIndicator: HTMLDivElement | null = null
  let horizontalIndicator: HTMLDivElement | null = null
  let hideTimer: number | undefined

  const hide = () => {
    verticalIndicator?.classList.remove('is-visible')
    horizontalIndicator?.classList.remove('is-visible')
  }

  document.addEventListener('scroll', event => {
    const element = scrollElement(event.target)
    if (!element) return

    const root = element === document.scrollingElement
    const rect = root
      ? { top: 0, right: window.innerWidth, bottom: window.innerHeight, left: 0, width: window.innerWidth, height: window.innerHeight }
      : element.getBoundingClientRect()
    const top = Math.max(0, rect.top)
    const left = Math.max(0, rect.left)
    const right = Math.min(window.innerWidth, rect.right)
    const bottom = Math.min(window.innerHeight, rect.bottom)
    const visibleWidth = Math.max(0, right - left)
    const visibleHeight = Math.max(0, bottom - top)
    const verticalInset = visibleHeight * (1 - trackScale) / 2
    const horizontalInset = visibleWidth * (1 - trackScale) / 2
    const trackHeight = visibleHeight * trackScale
    const trackWidth = visibleWidth * trackScale

    if (element.scrollHeight > element.clientHeight + 1 && visibleHeight > 0) {
      verticalIndicator ||= createIndicator('vertical')
      const thumbHeight = Math.min(trackHeight, Math.max(24, trackHeight * element.clientHeight / element.scrollHeight))
      const progress = element.scrollTop / Math.max(1, element.scrollHeight - element.clientHeight)
      verticalIndicator.style.left = `${Math.round(right - 5)}px`
      verticalIndicator.style.top = `${Math.round(top + verticalInset + (trackHeight - thumbHeight) * progress)}px`
      verticalIndicator.style.height = `${Math.round(thumbHeight)}px`
      verticalIndicator.classList.add('is-visible')
    } else {
      verticalIndicator?.classList.remove('is-visible')
    }

    if (element.scrollWidth > element.clientWidth + 1 && visibleWidth > 0) {
      horizontalIndicator ||= createIndicator('horizontal')
      const thumbWidth = Math.min(trackWidth, Math.max(24, trackWidth * element.clientWidth / element.scrollWidth))
      const progress = element.scrollLeft / Math.max(1, element.scrollWidth - element.clientWidth)
      horizontalIndicator.style.left = `${Math.round(left + horizontalInset + (trackWidth - thumbWidth) * progress)}px`
      horizontalIndicator.style.top = `${Math.round(bottom - 5)}px`
      horizontalIndicator.style.width = `${Math.round(thumbWidth)}px`
      horizontalIndicator.classList.add('is-visible')
    } else {
      horizontalIndicator?.classList.remove('is-visible')
    }

    window.clearTimeout(hideTimer)
    hideTimer = window.setTimeout(hide, hideDelayMs)
  }, { capture: true, passive: true })
}

installTransientScrollbars()
