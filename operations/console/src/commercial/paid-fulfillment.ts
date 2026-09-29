import { isLicenseRequestImage, readLicenseRequestImage } from '../license-request-import'

const MAX_REQUEST_BYTES = 16 * 1024

export async function readPaidLicenseRequest(file: File): Promise<string> {
  if (isLicenseRequestImage(file)) {
    const request = await readLicenseRequestImage(file)
    if (request.schema !== 'aster.license-request.v2') throw new Error('付费履约只接受 v2 安装请求')
    return JSON.stringify(request)
  }
  if (file.size < 1) throw new Error('安装请求文件为空')
  if (file.size > MAX_REQUEST_BYTES) throw new Error('安装请求文件不能超过 16 KiB')
  const bytes = new Uint8Array(await file.arrayBuffer())
  if (bytes.byteLength < 1 || bytes.byteLength > MAX_REQUEST_BYTES) throw new Error('安装请求文件大小无效')
  if (bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) throw new Error('安装请求文件不能包含 UTF-8 BOM')
  let text = ''
  try { text = new TextDecoder('utf-8', { fatal: true }).decode(bytes) }
  catch { throw new Error('安装请求文件必须是有效的 UTF-8 文本') }
  if (!text.trim()) throw new Error('安装请求文件为空')
  return text
}
