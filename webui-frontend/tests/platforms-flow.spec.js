import { expect, test } from '@playwright/test'

test('shows separate running platforms and saves QQ official credentials without changing OneBot', async ({page}) => {
  const napcat = {enabled:true,listen_addr:'127.0.0.1:6199',token:'***'}
  let config = {napcat,qq_official:{enabled:false,app_id:'',app_secret:''}}
  const writes = []
  await page.addInitScript(() => localStorage.setItem('yt_token','fixture'))
  await page.routeWebSocket('**/ws**', () => {})
  await page.route('**/api/**', async route => {
    const path = new URL(route.request().url()).pathname
    if (path === '/api/platforms') {
      return route.fulfill({json:{platforms:[
        {id:'onebot',state:'online',account:'9999'},
        {id:'qq_official',state:'disabled',account:null},
      ]}})
    }
    if (path === '/api/config' && route.request().method() === 'POST') {
      const next = route.request().postDataJSON().config
      writes.push(next)
      config = {...next,qq_official:{...next.qq_official,app_secret:'***'}}
      return route.fulfill({json:{ok:true,requires_restart:['qq_official（重启云团后生效）']}})
    }
    return route.fulfill({json:path === '/api/config' ? {config} : {}})
  })
  await page.setViewportSize({width:390,height:780})
  await page.goto('/platform')
  const onebot = page.locator('.n-card').filter({hasText:'OneBot 11'})
  const official = page.locator('.n-card').filter({hasText:'QQ 官方机器人'})
  await expect(onebot.getByText('已连接',{exact:true})).toBeVisible()
  await expect(onebot.getByText('9999',{exact:true})).toBeVisible()
  await expect(official.getByText('未启用',{exact:true})).toBeVisible()
  await official.getByRole('switch').click()
  await official.getByRole('textbox',{name:'AppID',exact:true}).fill('123456')
  const secret = official.getByLabel('AppSecret',{exact:true})
  await secret.fill('fixture-secret')
  await official.getByRole('button',{name:'保存配置',exact:true}).click()
  await expect(official.getByRole('status')).toHaveText('已保存，重启云团后生效。')
  expect(writes).toEqual([{napcat,qq_official:{enabled:true,app_id:'123456',app_secret:'fixture-secret'}}])
  await expect(secret).toHaveValue('')
  await expect(secret).toHaveAttribute('placeholder','已设置，留空保留原值')
  const links = official.getByRole('textbox',{name:'跨通道记忆关联'})
  await links.fill('member = 12345\nmember = 67890')
  await official.getByRole('button',{name:'保存配置',exact:true}).click()
  await expect(official.getByRole('status')).toHaveText('同一个 OpenID 只能关联一次')
  expect(writes).toHaveLength(1)
  await links.fill('member = 12345')
  await official.getByRole('button',{name:'保存配置',exact:true}).click()
  await expect(official.getByRole('status')).toHaveText('已保存，重启云团后生效。')
  expect(writes[1]).toEqual({napcat,qq_official:{enabled:true,app_id:'123456',app_secret:'***',memory_links:{member:'12345'}}})
  await expect(official.getByText('未启用',{exact:true})).toBeVisible()
  expect(await page.locator('.platform-grid').evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true)
})
