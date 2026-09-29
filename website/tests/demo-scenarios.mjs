import { expect } from '@playwright/test'

export async function checkDemo(page) {
  const demo = page.locator('[data-product-demo]')
  for (const locale of ['zh','en']) {
    const zh = locale === 'zh'
    await page.getByRole('button',{name:zh?'切换为简体中文':'Switch to English',exact:true}).click()
    const text = (a,b)=>zh?a:b
    await demo.getByRole('button',{name:text('重置体验','Reset demo'),exact:true}).click()
    const nav = demo.locator('.demo-sidebar nav')
    const choose = name=>nav.getByRole('button',{name,exact:true}).click()
    await expect(nav.getByRole('button')).toHaveText(zh?['运行概览','订阅/账号','成员与额度','全团队消费日志']:['Operations overview','Subscriptions & accounts','Members and quota','Team consumption logs'])
    await expect(demo.locator('.demo-metrics article').first()).toContainText('6.42M')
    await expect(demo.locator('[data-demo-account-count]')).toHaveText('3')
    await choose(text('订阅/账号','Subscriptions & accounts'))
    await expect(demo.getByRole('columnheader')).toHaveText(zh?['账号','提供方','套餐','凭据池','状态']:['Account','Provider','Plan','Credentials','Status'])
    for(let i=0;i<3;i++) await demo.getByRole('button',{name:text('新增示例账号','Add sample account'),exact:true}).click()
    await expect(demo.locator('.demo-pagination')).toContainText('6')
    await expect(demo.locator('.demo-table-row')).toHaveCount(1)
    await choose(text('运行概览','Operations overview'))
    await expect(demo.locator('[data-demo-account-count]')).toHaveText('6')
    await choose(text('成员与额度','Members and quota'))
    for(let i=0;i<3;i++) await demo.getByRole('button',{name:text('新增成员','Add member'),exact:true}).click()
    await expect(demo.locator('.demo-table-row')).toHaveCount(2)
    await demo.getByRole('button',{name:text('上一页','Previous'),exact:true}).click()
    await demo.locator('.demo-table-row').first().getByRole('button').click()
    await expect(demo.locator('.demo-table-row').first()).toContainText('8.86M')
    await choose(text('全团队消费日志','Team consumption logs'))
    await expect(demo.getByRole('columnheader')).toHaveText(zh?['时间','成员','模型','结算 Token','状态']:['Time','Member','Model','Billed tokens','Status'])
    await expect(demo.locator('.demo-pagination')).toContainText('21')
    await demo.getByRole('tab',{name:text('成员端','Member'),exact:true}).click()
    await expect(nav.getByRole('button')).toHaveText(zh?['工作台','API Key 管理','用量分析','消费日志']:['Dashboard','API Key Management','Usage Analytics','Consumption Logs'])
    await expect(demo.locator('[data-demo-balance]')).toContainText('8.86M')
    await choose(text('API Key 管理','API Key Management'))
    await demo.getByRole('button',{name:text('创建 Key','Create key'),exact:true}).click()
    await expect(demo.locator('.demo-key-value code')).toHaveCount(2)
    await demo.getByRole('button',{name:text('撤销示例 Key 4','Revoke sample key 4'),exact:true}).click()
    await expect(demo.locator('.demo-table-row').last()).toContainText(text('已撤销','Revoked'))
    await choose(text('用量分析','Usage Analytics'))
    await expect(demo.locator('.demo-daily-usage tbody tr')).toHaveCount(7)
    await expect(demo.locator('.demo-metrics')).toContainText('2.14M')
    await choose(text('消费日志','Consumption Logs'))
    let sum = 0
    for(let i=0;i<2;i++) {
      for(const row of await demo.locator('.demo-table-row').all()) {
        await expect(row.locator('td').nth(1)).toHaveText(text('林默','Alex Morgan'))
        sum += Number((await row.locator('td').nth(3).innerText()).replaceAll(',',''))
      }
      if(i===0) await demo.getByRole('button',{name:text('下一页','Next'),exact:true}).click()
    }
    expect(sum).toBe(2_140_000)
    await demo.getByRole('button',{name:text('重置体验','Reset demo'),exact:true}).click()
    await expect(demo.locator('[data-demo-account-count]')).toHaveText('3')
    await expect(demo.locator('[data-demo-member-count]')).toHaveText('4')
    await expect(demo.locator('[data-demo-key-count]')).toHaveText('3')
  }
}
