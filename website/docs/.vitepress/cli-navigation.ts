import type { DefaultTheme } from 'vitepress'

type CommandGroup = { zh: string; en: string; commands: string[] }
const clientGroups: CommandGroup[] = [
  { zh: '接入配置', en: 'Setup', commands: ['setup codex', 'setup claude'] },
  { zh: '状态与诊断', en: 'Status and diagnostics', commands: ['status codex', 'status claude', 'doctor codex', 'doctor claude'] },
  { zh: '移除与版本', en: 'Removal and version', commands: ['remove codex', 'remove claude', 'version'] },
]
const adminGroups: CommandGroup[] = [
  { zh: '安装与升级', en: 'Installation and upgrades', commands: ['install', 'upgrade'] },
  { zh: '运行与排查', en: 'Runtime and troubleshooting', commands: ['info', 'status', 'doctor', 'logs', 'trace', 'version'] },
  { zh: '服务管理', en: 'Service management', commands: ['service start', 'service stop', 'service restart'] },
  { zh: '许可证与管理员', en: 'Licensing and administrators', commands: ['license request', 'license install', 'license status', 'password reset-admin'] },
  { zh: '备份与恢复', en: 'Backup and restore', commands: ['backup create', 'backup restore'] },
  { zh: '独立 Runner', en: 'Dedicated Runner', commands: ['runner install', 'runner enroll', 'runner status', 'runner upgrade', 'runner backup create', 'runner backup restore'] },
  { zh: '卸载', en: 'Uninstall', commands: ['uninstall'] },
]

export function cliSidebar(language: 'zh-cn' | 'en'): DefaultTheme.SidebarItem[] {
  const zh = language === 'zh-cn'
  const base = `/${language}/tools`
  const commands = (tool: string, groups: CommandGroup[]): DefaultTheme.SidebarItem => ({
    text: zh ? '命令参考' : 'Command reference',
    link: `${base}/${tool}/commands`,
    collapsed: true,
    items: groups.map(group => ({
      text: zh ? group.zh : group.en,
      collapsed: true,
      items: group.commands.map(command => ({ text: command, link: `${base}/${tool}/${command.replaceAll(' ', '-')}` })),
    })),
  })
  return [
    { text: zh ? '客户端工具' : 'Client tools', collapsed: true, items: [
      { text: zh ? '概览与安装' : 'Overview and installation', link: `/${language}/guides/asterctl` },
      { text: zh ? '快速接入' : 'Quick setup', link: `${base}/asterctl/quickstart` },
      commands('asterctl', clientGroups),
      { text: zh ? '故障排查' : 'Troubleshooting', link: `${base}/asterctl/troubleshooting` },
    ] },
    { text: zh ? '管理员命令参考' : 'Administrator commands', collapsed: true, items: [
      { text: zh ? '概览与安装' : 'Overview and installation', link: `${base}/aster-team-cli/` },
      commands('aster-team-cli', adminGroups),
    ] },
  ]
}
