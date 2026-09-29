import { connect } from 'cloudflare:sockets'
import { buildLeadEmailContent } from './lead-email'
import type { ParsedLead } from './lead-model'

type SmtpConfig = {
  host: string
  port: number
  username: string
  password: string
  from: string
  to: string
}

const smtpTimeoutMs = 10_000
const emailPattern = /^[^\s@<>]+@[^\s@<>]+\.[^\s@<>]+$/

function sanitizeHeader(value: string): string {
  return value.replace(/[\r\n]+/g, ' ').trim()
}

function utf8Base64(value: string): string {
  const bytes = new TextEncoder().encode(value)
  let binary = ''
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary)
}

function encodedHeader(value: string): string {
  return `=?UTF-8?B?${utf8Base64(value)}?=`
}

function dotStuff(value: string): string {
  return value.replace(/\r?\n/g, '\r\n').split('\r\n').map(line => line.startsWith('.') ? `.${line}` : line).join('\r\n')
}

function withTimeout<T>(promise: Promise<T>, message: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(message)), smtpTimeoutMs)
  })
  return Promise.race([promise, timeout]).finally(() => {
    if (timer !== undefined) clearTimeout(timer)
  })
}

class SmtpSession {
  readonly #reader: ReadableStreamDefaultReader<Uint8Array>
  readonly #writer: WritableStreamDefaultWriter<Uint8Array>
  readonly #decoder = new TextDecoder()
  readonly #encoder = new TextEncoder()
  #buffer = ''

  constructor(readable: ReadableStream, writable: WritableStream) {
    this.#reader = readable.getReader() as ReadableStreamDefaultReader<Uint8Array>
    this.#writer = writable.getWriter() as WritableStreamDefaultWriter<Uint8Array>
  }

  async command(value: string, allowedCodes: readonly number[], stage: string): Promise<void> {
    await withTimeout(this.#writer.write(this.#encoder.encode(`${value}\r\n`)), `SMTP ${stage} write timed out`)
    await this.expect(allowedCodes, stage)
  }

  async data(value: string): Promise<void> {
    await withTimeout(this.#writer.write(this.#encoder.encode(`${value}\r\n.\r\n`)), 'SMTP body write timed out')
    await this.expect([250], 'message acceptance')
  }

  async expect(allowedCodes: readonly number[], stage: string): Promise<void> {
    const firstLine = await this.readLine()
    const code = Number(firstLine.slice(0, 3))
    if (!Number.isInteger(code)) throw new Error(`SMTP ${stage} returned an invalid response`)
    if (firstLine[3] === '-') {
      const terminator = `${code} `
      let line = firstLine
      while (!line.startsWith(terminator)) line = await this.readLine()
    }
    if (!allowedCodes.includes(code)) throw new Error(`SMTP ${stage} failed with status ${code}`)
  }

  async close(): Promise<void> {
    await this.#reader.cancel().catch(() => undefined)
    await this.#writer.close().catch(() => undefined)
  }

  private async readLine(): Promise<string> {
    while (true) {
      const lineEnd = this.#buffer.indexOf('\r\n')
      if (lineEnd >= 0) {
        const line = this.#buffer.slice(0, lineEnd)
        this.#buffer = this.#buffer.slice(lineEnd + 2)
        return line
      }
      const result = await withTimeout(this.#reader.read(), 'SMTP response timed out')
      if (result.done) throw new Error('SMTP connection closed unexpectedly')
      this.#buffer += this.#decoder.decode(result.value, { stream: true })
      if (this.#buffer.length > 64 * 1024) throw new Error('SMTP response was too large')
    }
  }
}

type WebsiteEmail = { subject: string; body: string; contact: string }

function buildMessage(config: SmtpConfig, id: string, message: WebsiteEmail, createdAt: string): string {
  const { subject, body, contact } = message
  const senderDomain = config.from.split('@')[1]
  const headers = [
    `From: Aster Team Website <${sanitizeHeader(config.from)}>`,
    `To: ${sanitizeHeader(config.to)}`,
    `Subject: ${encodedHeader(subject)}`,
    `Date: ${new Date(createdAt).toUTCString()}`,
    `Message-ID: <${sanitizeHeader(id)}@${senderDomain}>`,
    'MIME-Version: 1.0',
    'Content-Type: text/plain; charset=UTF-8',
    'Content-Transfer-Encoding: 8bit',
  ]
  if (emailPattern.test(contact)) headers.push(`Reply-To: ${sanitizeHeader(contact)}`)
  return dotStuff(`${headers.join('\r\n')}\r\n\r\n${body}`)
}

export async function sendLeadEmail(config: SmtpConfig, id: string, lead: ParsedLead, createdAt: string): Promise<void> {
  return sendWebsiteEmail(config, id, { ...buildLeadEmailContent(id, lead, createdAt), contact: lead.contact }, createdAt)
}

export async function sendWebsiteEmail(config: SmtpConfig, id: string, message: WebsiteEmail, createdAt: string): Promise<void> {
  if (!emailPattern.test(config.from) || !emailPattern.test(config.to) || !config.host || !config.username || !config.password) {
    throw new Error('SMTP configuration is incomplete')
  }

  const socket = connect({ hostname: config.host, port: config.port }, { secureTransport: 'on', allowHalfOpen: false })
  let session: SmtpSession | undefined
  try {
    await withTimeout(socket.opened, 'SMTP connection timed out')
    session = new SmtpSession(socket.readable, socket.writable)
    await session.expect([220], 'greeting')
    await session.command(`EHLO ${config.from.split('@')[1]}`, [250], 'EHLO')
    await session.command('AUTH LOGIN', [334], 'authentication')
    await session.command(utf8Base64(config.username), [334], 'username authentication')
    await session.command(utf8Base64(config.password), [235], 'password authentication')
    await session.command(`MAIL FROM:<${config.from}>`, [250], 'sender')
    await session.command(`RCPT TO:<${config.to}>`, [250, 251], 'recipient')
    await session.command('DATA', [354], 'message data')
    await session.data(buildMessage(config, id, message, createdAt))
    await session.command('QUIT', [221], 'QUIT')
  } finally {
    await session?.close()
    await socket.close().catch(() => undefined)
  }
}
