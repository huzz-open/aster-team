import { isIP } from 'node:net'

export function localLanAccessEnabled(environment = process.env) {
  return environment.ASTER_LOCAL_LAN_ENABLED === 'true'
}

export function localDevelopmentBindHost(environment = process.env) {
  return localLanAccessEnabled(environment) ? '0.0.0.0' : '127.0.0.1'
}

export function localDevelopmentAdvertisedHost(environment = process.env) {
  if (!localLanAccessEnabled(environment)) return '127.0.0.1'
  const host = environment.ASTER_LOCAL_LAN_HOST?.trim()
  if (!host || isIP(host) !== 4 || host.startsWith('127.') || host.startsWith('169.254.')) {
    throw new Error('ASTER_LOCAL_LAN_HOST must be a non-loopback IPv4 address')
  }
  return host
}
