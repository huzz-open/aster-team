export const OPERATIONS_SESSION_EXPIRED_EVENT = 'aster:operations-session-expired'

export function notifyOperationsSessionExpired(): void {
  if (typeof window !== 'undefined') window.dispatchEvent(new Event(OPERATIONS_SESSION_EXPIRED_EVENT))
}
