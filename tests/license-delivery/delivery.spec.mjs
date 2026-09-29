import { expect, test } from '@playwright/test'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import QRCode from 'qrcode'

import {
  compactLicenseRequest, licenseRequest,
} from './fixtures.mjs'

const operationsURL = `http://127.0.0.1:${process.env.ASTER_LICENSE_OPERATIONS_PORT ?? '12080'}`

async function writeTerminalScreenshot(page, path) {
  const qr = QRCode.create([{ data: compactLicenseRequest(), mode: 'byte' }], { errorCorrectionLevel: 'L' })
  const size = qr.modules.size
  const modules = Array.from(qr.modules.data, Boolean)
  await page.setContent('<canvas id="terminal"></canvas>')
  const dataURL = await page.evaluate(({ modules, size }) => {
    const cellWidth = 18
    const cellHeight = 19
    const border = 1
    const headingHeight = 34
    const canvas = document.querySelector('#terminal')
    canvas.width = (size + border * 2) * cellWidth
    canvas.height = headingHeight + (size + border * 2) * cellHeight
    const context = canvas.getContext('2d')
    context.fillStyle = '#05070b'
    context.fillRect(0, 0, canvas.width, canvas.height)
    context.fillStyle = '#f7f7f5'
    context.font = '16px monospace'
    context.fillText('Machine authorization request QR:', 4, 20)
    for (let y = 0; y < size; y += 1) {
      for (let x = 0; x < size; x += 1) {
        if (!modules[y * size + x]) continue
        context.fillRect((x + border) * cellWidth, headingHeight + (y + border) * cellHeight, cellWidth, cellHeight)
      }
    }
    return canvas.toDataURL('image/png')
  }, { modules, size })
  await writeFile(path, Buffer.from(dataURL.slice(dataURL.indexOf(',') + 1), 'base64'))
}

async function writeTransportFixture(page, transport, directory) {
  await mkdir(directory, { recursive: true })
  if (transport === 'json') {
    const path = `${directory}/machine-request.json`
    await writeFile(path, `${JSON.stringify(licenseRequest, null, 2)}\n`)
    return path
  }
  if (transport === 'qr-png') {
    const path = `${directory}/machine-request.qr.png`
    await QRCode.toFile(path, [{ data: compactLicenseRequest(), mode: 'byte' }], {
      errorCorrectionLevel: 'L', margin: 4, scale: 20,
    })
    return path
  }
  const path = `${directory}/terminal-screenshot.png`
  await writeTerminalScreenshot(page, path)
  return path
}

// The retired v1 issuance page is intentionally absent. Exercise the real
// browser import module; signed issuance and installation have separate real
// Operations/Customer integration suites.
for (const transport of ['json', 'qr-png', 'terminal-screenshot']) {
  test(`v2 installation request is preserved through ${transport}`, async ({ page }, testInfo) => {
    const path = await writeTransportFixture(page, transport, testInfo.outputPath('transport'))
    const bytes = [...await readFile(path)]
    await page.goto(`${operationsURL}/login`)
    const text = await page.evaluate(async ({ bytes, transport }) => {
      const { readPaidLicenseRequest } = await import('/src/commercial/paid-fulfillment.ts')
      const type = transport === 'json' ? 'application/json' : 'image/png'
      const name = transport === 'json' ? 'request.json' : 'request.png'
      return readPaidLicenseRequest(new File([new Uint8Array(bytes)], name, { type }))
    }, { bytes, transport })
    expect(JSON.parse(text)).toEqual(licenseRequest)
    if (transport === 'json') expect(text).toBe(await readFile(path, 'utf8'))
  })
}
