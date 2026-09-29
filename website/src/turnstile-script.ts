let loading: Promise<void> | null = null

export function loadTurnstile(): Promise<void> {
  if (window.turnstile) return Promise.resolve()
  if (loading) return loading
  loading = new Promise<void>((resolve, reject) => {
    const existing = document.querySelector<HTMLScriptElement>('script[data-aster-turnstile]')
    const script = existing ?? document.createElement('script')
    const cleanup = () => {
      window.clearTimeout(timer)
      script.removeEventListener('load', loaded)
      script.removeEventListener('error', failed)
    }
    const failed = () => { cleanup(); script.remove(); reject(new Error('Turnstile script is unavailable')) }
    const loaded = () => {
      if (!window.turnstile) { failed(); return }
      cleanup()
      resolve()
    }
    const timer = window.setTimeout(failed, 10_000)
    script.addEventListener('load', loaded, { once: true })
    script.addEventListener('error', failed, { once: true })
    if (!existing) {
      script.src = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit'
      script.async = true
      script.defer = true
      script.dataset.asterTurnstile = 'true'
      document.head.append(script)
    }
  }).finally(() => { loading = null })
  return loading
}
