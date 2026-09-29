import { defineConfig } from 'vitepress'
import { cliSidebar } from './cli-navigation'
import { administrationSidebar, validateManualGuides } from '../../build/manual-guides.mjs'

validateManualGuides()

const localDocumentation = process.env.ASTER_DOCS_TARGET === 'customer'

const zhSidebar = [
  { text: '开始使用', items: [
    { text: '概览', link: '/zh-cn/' },
    { text: '快速开始', link: '/zh-cn/start/quickstart' },
    { text: '身份认证', link: '/zh-cn/start/authentication' },
  ] },
  administrationSidebar('zh-cn'),
  { text: 'API 参考', items: [
    { text: 'Responses', link: '/zh-cn/api/responses' },
    { text: 'Chat Completions', link: '/zh-cn/api/chat-completions' },
    { text: 'Anthropic Messages', link: '/zh-cn/api/messages' },
    { text: '图片生成与编辑', link: '/zh-cn/api/images' },
  ] },
  { text: '模型与参数', items: [
    { text: '模型列表', link: '/zh-cn/models/' },
    { text: '模型能力矩阵', link: '/zh-cn/models/capability-matrix' },
    { text: 'deepseek-v4-pro', link: '/zh-cn/models/deepseek-v4-pro' },
    { text: 'deepseek-flash', link: '/zh-cn/models/deepseek-flash' },
    { text: 'glm-5.2', link: '/zh-cn/models/glm-5.2' },
  ] },
  { text: '开发与排查', items: [
    { text: 'SDK 示例', link: '/zh-cn/guides/sdk-examples' },
    { text: '错误码', link: '/zh-cn/guides/errors' },
    { text: '常见问题', link: '/zh-cn/guides/faq' },
  ] },
  ...cliSidebar('zh-cn'),
]

const enSidebar = [
  { text: 'Get started', items: [
    { text: 'Overview', link: '/en/' },
    { text: 'Quickstart', link: '/en/start/quickstart' },
    { text: 'Authentication', link: '/en/start/authentication' },
  ] },
  administrationSidebar('en'),
  { text: 'API reference', items: [
    { text: 'Responses', link: '/en/api/responses' },
    { text: 'Chat Completions', link: '/en/api/chat-completions' },
    { text: 'Anthropic Messages', link: '/en/api/messages' },
    { text: 'Image generation and editing', link: '/en/api/images' },
  ] },
  { text: 'Models and fields', items: [
    { text: 'Model list', link: '/en/models/' },
    { text: 'Capability matrix', link: '/en/models/capability-matrix' },
    { text: 'deepseek-v4-pro', link: '/en/models/deepseek-v4-pro' },
    { text: 'deepseek-flash', link: '/en/models/deepseek-flash' },
    { text: 'glm-5.2', link: '/en/models/glm-5.2' },
  ] },
  { text: 'Develop and troubleshoot', items: [
    { text: 'SDK examples', link: '/en/guides/sdk-examples' },
    { text: 'Errors', link: '/en/guides/errors' },
    { text: 'FAQ', link: '/en/guides/faq' },
  ] },
  ...cliSidebar('en'),
]

export default defineConfig({
  base: '/docs/',
  vite: { define: { __ASTER_DOCS_LOCAL__: JSON.stringify(localDocumentation) } },
  lang: 'en-US',
  title: 'Aster Team',
  description: 'Aster Team installation, administration, troubleshooting and API documentation',
  cleanUrls: true,
  sitemap: localDocumentation ? undefined : {
    hostname: 'https://aster.huzz.top/docs/',
    transformItems: items => items
      .filter(item => /^(zh-cn|en)\//.test(item.url))
      .map(item => ({ ...item, links: item.links?.filter(link => /^(zh-cn|en)\//.test(link.url)) })),
  },
  transformHead({ pageData }) {
    const source = pageData.relativePath
    // Bundled documentation describes its installed version, which may differ from the website.
    if (localDocumentation || !/^(zh-cn|en)\//.test(source)) {
      return [['meta', { name: 'robots', content: 'noindex, follow' }]]
    }
    const route = source.replace(/(?:index)?\.md$/, '')
    const canonical = `https://aster.huzz.top/docs/${route}`
    const counterpart = route.replace(/^(zh-cn|en)\//, source.startsWith('en/') ? 'zh-cn/' : 'en/')
    return [
      ['link', { rel: 'canonical', href: canonical }],
      ['link', { rel: 'alternate', hreflang: source.startsWith('en/') ? 'en' : 'zh-CN', href: canonical }],
      ['link', { rel: 'alternate', hreflang: source.startsWith('en/') ? 'zh-CN' : 'en', href: `https://aster.huzz.top/docs/${counterpart}` }],
      ['link', { rel: 'alternate', hreflang: 'x-default', href: `https://aster.huzz.top/docs/${route.replace(/^zh-cn\//, 'en/')}` }],
    ]
  },
  markdown: { theme: { light: 'github-light', dark: 'github-dark' } },
  themeConfig: { search: { provider: 'local' }, outline: { level: [2, 3] } },
  locales: {
    'zh-cn': {
      label: '简体中文', lang: 'zh-CN', link: '/zh-cn/',
      themeConfig: {
        nav: [{ text: '产品文档', link: '/zh-cn/' }, { text: '官网', link: 'https://aster.huzz.top/' }],
        sidebar: zhSidebar,
        outline: { level: [2, 3], label: '本页目录' },
        docFooter: { prev: '上一页', next: '下一页' },
        sidebarMenuLabel: '菜单',
        returnToTopLabel: '返回顶部',
        langMenuLabel: '切换语言',
        skipToContentLabel: '跳转到内容',
        darkModeSwitchLabel: '外观',
        darkModeSwitchTitle: '切换到深色主题',
        lightModeSwitchTitle: '切换到浅色主题',
        notFound: { title: '页面不存在', quote: '请检查地址，或返回文档首页。', linkLabel: '返回文档首页', linkText: '返回文档首页' },
        search: {
          provider: 'local',
          options: {
            translations: {
              button: { buttonText: '搜索文档', buttonAriaLabel: '搜索文档' },
              modal: {
                displayDetails: '显示详细结果',
                resetButtonTitle: '清空搜索',
                backButtonTitle: '关闭搜索',
                noResultsText: '未找到相关结果',
                footer: { selectText: '选择', selectKeyAriaLabel: '回车', navigateText: '切换', navigateUpKeyAriaLabel: '上方向键', navigateDownKeyAriaLabel: '下方向键', closeText: '关闭', closeKeyAriaLabel: 'Esc' },
              },
            },
          },
        },
      },
    },
    en: {
      label: 'English', lang: 'en-US', link: '/en/',
      themeConfig: { nav: [{ text: 'Product docs', link: '/en/' }, { text: 'Website', link: 'https://aster.huzz.top/' }], sidebar: enSidebar },
    },
  },
})
