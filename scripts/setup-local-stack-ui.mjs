import { setupLocalStack } from './setup-local-stack.mjs'

if (process.env.ASTER_LOCAL_UI_CONFIRMED !== 'true') {
  throw new Error('此命令只能在本地开发 UI 明确确认后执行')
}

await setupLocalStack({
  args: [],
  interactive: true,
  confirm: async () => {
    console.log('图形界面确认已完成，开始重建本地数据库和运行配置。')
    return true
  },
  log: message => {
    if (/^\s*(Customer|Operations) admin:/.test(message)) {
      console.log(message.replace(/\s\/\s.+$/, ' / [已保存到本地管理员账号文件]'))
    } else {
      console.log(message)
    }
  },
})
