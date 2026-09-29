import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const websiteRoot = fileURLToPath(new URL('../', import.meta.url))
const text = path => readFileSync(new URL(path, `file:///${websiteRoot.replaceAll('\\', '/')}/`), 'utf8')

test('public shell exposes useful product content and canonical discovery metadata without JavaScript', () => {
  const html = text('index.html')
  assert.match(html, /<title>Aster Team \| Your private AI gateway<\/title>/)
  assert.match(html, /<link rel="canonical" href="https:\/\/aster\.huzz\.top\/">/)
  assert.match(html, /<meta name="description" content="[^"]*privately deployed AI gateway[^"]*subscription accounts[^"]*">/)
  assert.match(html, /<div id="app"><\/div><noscript><main class="site-noscript">/)
  assert.match(html, /<h1>Empower your team with AI\. Keep every investment in control\.<\/h1>/)
  assert.match(html, /html,body,#app\{[^}]*min-height:100%[^}]*background:#f6f4ee/)
  assert.doesNotMatch(html, /<div id="app"><main/)

  const robots = text('public/robots.txt')
  const sitemap = text('public/sitemap.xml')
  assert.match(robots, /Sitemap: https:\/\/aster\.huzz\.top\/sitemap\.xml/)
  assert.match(sitemap, /<loc>https:\/\/aster\.huzz\.top\/<\/loc>/)
})

test('social preview has the declared 1200 by 630 dimensions', () => {
  const image = readFileSync(new URL('public/og.png', `file:///${websiteRoot.replaceAll('\\', '/')}/`))
  assert.equal(image.subarray(1, 4).toString('ascii'), 'PNG')
  assert.equal(image.readUInt32BE(16), 1200)
  assert.equal(image.readUInt32BE(20), 630)
})
