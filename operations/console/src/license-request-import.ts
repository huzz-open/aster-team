import { BrowserQRCodeReader } from '@zxing/browser'
import { DecodeHintType, ResultMetadataType, type Result } from '@zxing/library'

import type { PaidLicenseRequest } from './api/client'
import { CAPABILITY_CATALOG_VERSION } from './api/generated/product-capabilities'

const MAX_IMAGE_BYTES = 10 * 1024 * 1024
const MAX_IMAGE_PIXELS = 16 * 1024 * 1024
const COMPACT_QR_PREFIX = new Uint8Array([0x41, 0x4c, 0x52])
const COMPACT_QR_V2 = 0x03
const COMPACT_QR_V2_FIXED_BYTES = 123
const QUOTA_POLICY_VERSION = 1 as const
const TERMINAL_SCREEN_HEIGHT_SCALES = [1, 18 / 19] as const
const QR_DECODE_MAX_DIMENSIONS = [512, 1024, 1600] as const
const QR_DECODE_QUIET_PADDING_RATIO = 0.08

function base64URL(bytes: Uint8Array): string {
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
}

async function sha256Base64URL(value: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(value))
  return base64URL(new Uint8Array(digest))
}

function qrBytePayload(result: Result): Uint8Array {
  const segments = result.getResultMetadata()?.get(ResultMetadataType.BYTE_SEGMENTS)
  if (!Array.isArray(segments) || segments.length !== 1 || !(segments[0] instanceof Uint8Array)) {
    throw new Error('二维码不是当前版本的 Aster Team 机器授权申请')
  }
  return segments[0]
}

export async function parseCompactQRPayload(payload: Uint8Array): Promise<PaidLicenseRequest> {
  if (payload.length < COMPACT_QR_PREFIX.length + 1) {
    throw new Error('二维码不是当前版本的 Aster Team 机器授权申请')
  }
  for (let index = 0; index < COMPACT_QR_PREFIX.length; index += 1) {
    if (payload[index] !== COMPACT_QR_PREFIX[index]) {
      throw new Error('二维码不是当前版本的 Aster Team 机器授权申请')
    }
  }
  const wireVersion = payload[3]
  if (wireVersion !== COMPACT_QR_V2) {
    throw new Error('二维码不是当前版本的 Aster Team 机器授权申请')
  }
  const fixedBytes = COMPACT_QR_V2_FIXED_BYTES
  if (payload.length < fixedBytes + 1) throw new Error('机器授权申请二维码长度无效')

  const platform = ({ 1: 'linux', 2: 'windows', 3: 'macos' } as const)[payload[4] as 1 | 2 | 3]
  const architecture = ({ 1: 'amd64', 2: 'arm64' } as const)[payload[5] as 1 | 2]
  if (!platform || !architecture) throw new Error('机器授权申请二维码平台或架构无效')
  const versionLength = payload[6]!
  if (versionLength < 1 || versionLength > 64 || payload.length !== fixedBytes + versionLength) {
    throw new Error('机器授权申请二维码长度无效')
  }

  const view = new DataView(payload.buffer, payload.byteOffset, payload.byteLength)
  const capabilityCatalogVersion = view.getUint32(7, false)
  const quotaPolicyVersion = view.getUint32(11, false)
  const generatedAtOffset = 15
  const requestOffset = 23
  const installationOffset = 41
  const dmiOffset = 59
  const machineOffset = 91
  const generatedAtMillis = view.getBigUint64(generatedAtOffset, false)
  if (generatedAtMillis > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error('机器授权申请二维码时间无效')
  const generatedAtDate = new Date(Number(generatedAtMillis))
  if (!Number.isFinite(generatedAtDate.getTime())) throw new Error('机器授权申请二维码时间无效')

  let productVersion: string
  try {
    productVersion = new TextDecoder('utf-8', { fatal: true }).decode(payload.subarray(fixedBytes))
  } catch {
    throw new Error('二维码不是当前版本的 Aster Team 机器授权申请')
  }
  if (!/^[A-Za-z0-9._:@+/-]+$/.test(productVersion)) throw new Error('机器授权申请二维码产品版本无效')

  const requestID = `request_${base64URL(payload.subarray(requestOffset, requestOffset + 18))}`
  const installationID = `installation_${base64URL(payload.subarray(installationOffset, installationOffset + 18))}`
  const dmiUUID = base64URL(payload.subarray(dmiOffset, dmiOffset + 32))
  const machineID = base64URL(payload.subarray(machineOffset, machineOffset + 32))
  const fingerprint = await sha256Base64URL(
    `aster-team\n${installationID}\ndmi_product_uuid=${dmiUUID}\nmachine_id=${machineID}\n`,
  )

  const base = {
    request_id: requestID,
    product: 'aster-team',
    product_version: productVersion,
    platform,
    architecture,
    installation_id: installationID,
    machine_fingerprint_sha256: fingerprint,
    machine_factors: [
      { kind: 'dmi_product_uuid', sha256: dmiUUID },
      { kind: 'machine_id', sha256: machineID },
    ],
    generated_at: generatedAtDate.toISOString(),
  } satisfies Omit<PaidLicenseRequest, 'schema' | 'license_schema' | 'capability_catalog_version' | 'quota_policy_version'>
  if (capabilityCatalogVersion !== CAPABILITY_CATALOG_VERSION) {
    throw new Error('机器授权申请二维码协议版本无效')
  }
  if (quotaPolicyVersion !== QUOTA_POLICY_VERSION) {
    throw new Error('机器授权申请二维码协议版本无效')
  }
  return {
    ...base,
    schema: 'aster.license-request.v2',
    license_schema: 'aster.license.v2',
    capability_catalog_version: capabilityCatalogVersion,
    quota_policy_version: quotaPolicyVersion,
  } satisfies PaidLicenseRequest
}

export function isLicenseRequestImage(file: File): boolean {
  return file.type.startsWith('image/') || /\.(?:png|jpe?g|webp)$/i.test(file.name)
}

export async function readLicenseRequestImage(file: File): Promise<PaidLicenseRequest> {
  if (file.size > MAX_IMAGE_BYTES) throw new Error('二维码图片不能超过 10 MiB')
  if (typeof createImageBitmap !== 'function') throw new Error('当前浏览器不支持本地二维码图片解析，请改用 JSON 申请文件')
  const bitmap = await createImageBitmap(file)
  try {
    if (bitmap.width < 1 || bitmap.height < 1 || bitmap.width * bitmap.height > MAX_IMAGE_PIXELS) {
      throw new Error('二维码图片尺寸无效或超过 1600 万像素')
    }
    const canvas = document.createElement('canvas')
    const context = canvas.getContext('2d', { willReadFrequently: true })
    if (!context) throw new Error('浏览器无法读取二维码图片')
    const hints = new Map<DecodeHintType, unknown>([[DecodeHintType.TRY_HARDER, true]])
    const reader = new BrowserQRCodeReader(hints)
    let result: Result | undefined
    for (const maximumDimension of QR_DECODE_MAX_DIMENSIONS) {
      const imageScale = Math.min(1, maximumDimension / Math.max(bitmap.width, bitmap.height))
      for (const heightScale of TERMINAL_SCREEN_HEIGHT_SCALES) {
        for (const inverted of [false, true]) {
          const imageWidth = Math.max(1, Math.round(bitmap.width * imageScale))
          const imageHeight = Math.max(1, Math.round(bitmap.height * imageScale * heightScale))
          const quietPadding = Math.max(8, Math.round(Math.min(imageWidth, imageHeight) * QR_DECODE_QUIET_PADDING_RATIO))
          canvas.width = imageWidth + quietPadding * 2
          canvas.height = imageHeight + quietPadding * 2
          context.imageSmoothingEnabled = false
          context.globalCompositeOperation = 'source-over'
          context.fillStyle = inverted ? '#000' : '#fff'
          context.fillRect(0, 0, canvas.width, canvas.height)
          context.drawImage(bitmap, quietPadding, quietPadding, imageWidth, imageHeight)
          if (inverted) {
            context.globalCompositeOperation = 'difference'
            context.fillStyle = '#fff'
            context.fillRect(0, 0, canvas.width, canvas.height)
            context.globalCompositeOperation = 'source-over'
          }
          try {
            result = reader.decodeFromCanvas(canvas)
            break
          } catch {
            // Continue through bounded size, aspect and colour retries. Decoding
            // the original multi-megapixel photo with TRY_HARDER can block the UI.
          }
        }
        if (result) break
      }
      if (result) break
    }
    if (!result) throw new Error('图片中没有识别到机器授权申请二维码；请确保截图包含二维码四周留白')
    return parseCompactQRPayload(qrBytePayload(result))
  } finally {
    bitmap.close()
  }
}
