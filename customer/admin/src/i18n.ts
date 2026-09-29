import { ref } from 'vue'

export type Locale = 'zh-CN' | 'en-US'
const initial = localStorage.getItem('aster-locale') === 'en-US' ? 'en-US' : 'zh-CN'
export const locale = ref<Locale>(initial)

const messages = {
  'zh-CN': {
    administrator: '管理员',
    workspace: '客户管理控制台', operation: '运行管理', resources: '资源与接入', security: '安全与授权', overview: '运行概览', consumptionLogs: '全团队消费日志', auditEvents: '安全审计', users: '成员管理', billing: '费用', vouchers: '兑换券', quotaRequests: '金额申请审批', models: '可用模型', accounts: '订阅/账号', runners: 'Runner 节点', license: '产品授权', maintenance: '系统升级', settings: '系统管理',
    notice: '系统通知', theme: '切换主题', language: '切换语言', profile: '管理员账户', logout: '退出管理端', collapse: '收起导航', allClear: '当前没有需要处理的系统通知。',
  },
  'en-US': {
    administrator: 'Administrator',
    workspace: 'Customer Admin', operation: 'Operations', resources: 'Resources & Access', security: 'Security & License', overview: 'Operations overview', consumptionLogs: 'Team consumption logs', auditEvents: 'Security audit', users: 'Members', billing: 'Billing', vouchers: 'Vouchers', quotaRequests: 'Amount request reviews', models: 'Available models', accounts: 'Subscriptions & accounts', runners: 'Runner nodes', license: 'Product license', maintenance: 'System upgrade', settings: 'System management',
    notice: 'System notices', theme: 'Switch theme', language: 'Change language', profile: 'Administrator', logout: 'Sign out', collapse: 'Collapse sidebar', allClear: 'There are no system notices requiring your attention.',
  },
} as const

export type MessageKey = keyof typeof messages['zh-CN']
export function t(key: MessageKey) { return messages[locale.value][key] }
export function setLocale(value: Locale) { locale.value = value; localStorage.setItem('aster-locale', value); document.documentElement.lang = value }
setLocale(initial)
