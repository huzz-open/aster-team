import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export const manualTopics = [
  ['preparation', '安装前准备', 'Before you install'],
  ['installation', '安装部署', 'Installation'],
  ['licensing', '产品授权', 'Product authorization'],
  ['management', '管理与使用', 'Manage model access'],
  ['runners', 'Runner 节点', 'Runner nodes'],
  ['daily-checks', '日常检查', 'Daily checks'],
  ['backup-upgrade', '备份、升级与恢复', 'Backup, upgrade and restore'],
  ['troubleshooting', '故障排查', 'Troubleshooting'],
]

export function manualRegion(source, name) {
  const lines = source.replaceAll('\r\n', '\n').split('\n')
  const starts = lines.flatMap((line, index) => line === `<!-- #region ${name} -->` ? [index] : [])
  const ends = lines.flatMap((line, index) => line === `<!-- #endregion ${name} -->` ? [index] : [])
  if (starts.length !== 1 || ends.length !== 1 || ends[0] <= starts[0]) {
    throw new Error(`user-manual.md: expected one complete region ${name}`)
  }
  const content = lines.slice(starts[0] + 1, ends[0]).join('\n').trim()
  if (!content || /<!--\s*(?:@include:|#(?:end)?region)/.test(content)) {
    throw new Error(`user-manual.md: empty or nested region ${name}`)
  }
  // Included links resolve from the public page, not from docs/user-manual.md.
  if (/\]\(\s*(?:\.\.?\/|[^/\s:)]+\.md(?:[#)]))/.test(content)) {
    throw new Error(`user-manual.md: region ${name} contains a repository-relative link`)
  }
  return content
}

export function manualSourceHash(content) {
  return createHash('sha256').update(content.replaceAll('\r\n', '\n').trim()).digest('hex')
}

export function validateManualGuides(root = resolve(dirname(fileURLToPath(import.meta.url)), '../..')) {
  const manual = readFileSync(resolve(root, 'docs/user-manual.md'), 'utf8')
  for (const [slug] of manualTopics) {
    const content = manualRegion(manual, slug)
    const chinese = readFileSync(resolve(root, `website/docs/zh-cn/administration/${slug}.md`), 'utf8')
    const expected = `<!--@include: ../../../../docs/user-manual.md#${slug}-->`
    if (!chinese.includes(expected) || [...chinese.matchAll(/<!--\s*@include:/g)].length !== 1) {
      throw new Error(`${slug}: Chinese guide must include its canonical manual region exactly once`)
    }
    const english = readFileSync(resolve(root, `website/docs/en/administration/${slug}.md`), 'utf8')
    const frontmatter = english.match(/^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/)?.[1] ?? ''
    const reviewedHash = frontmatter.match(/^manualSourceHash: ([a-f0-9]{64})\r?$/m)?.[1]
    if (reviewedHash !== manualSourceHash(content)) {
      throw new Error(`${slug}: review the English guide against the changed manual region, then update manualSourceHash`)
    }
  }
}

export function administrationSidebar(language) {
  const zh = language === 'zh-cn'
  return {
    text: zh ? '部署与运维' : 'Deployment and operations',
    collapsed: true,
    items: [
      { text: zh ? '部署路线' : 'Deployment roadmap', link: `/${language}/administration/` },
      ...manualTopics.map(([slug, chinese, english]) => ({
        text: zh ? chinese : english,
        link: `/${language}/administration/${slug}`,
      })),
    ],
  }
}
