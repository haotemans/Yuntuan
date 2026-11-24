import { expect, test } from '@playwright/test'

async function setup(page, model = true) {
  await page.addInitScript(() => localStorage.setItem('yt_token', 'test-only'))
  await page.routeWebSocket('**/ws**', () => {})
  const jobs = [], writes = []
  await page.route('**/api/**', async route => {
    const req=route.request(), path=new URL(req.url()).pathname
    if(path==='/api/runtime/agent') return route.fulfill({json:{model_configured:model,bash:{state:'disabled',message:'Bash 未启用，请先部署沙箱'},tools:[{name:'sandbox_exec',available:true}]}})
    if(path==='/api/runtime/bash/check') return route.fulfill({json:{ok:false,state:'environment_missing',message:'Docker 或沙箱镜像不可用，请检查连接'}})
    if(path==='/api/agent-schedules') {
      if(req.method()==='GET')return route.fulfill({json:{tasks:jobs}})
      const b=req.postDataJSON(); writes.push(b)
      jobs.push({...b,id:1,enabled:true,local_hour:9,local_minute:15,next_run_at:1900000000})
    } else if(path.endsWith('/history')) return route.fulfill({json:{runs:[{id:1,task_id:'scheduled-1',triggered_at:1890000000,phase:'failed',error:'命令执行超时'}]}})
    else if(path.endsWith('/run')) {Object.assign(jobs[0],{task_id:'scheduled-1',phase:'queued',triggered_at:1890000000});return route.fulfill({json:{message:'已进入任务队列，尚未执行完成'}})}
    else if(path.endsWith('/time')) {Object.assign(jobs[0],{local_hour:8,local_minute:30})}
    else if(path==='/api/agent-schedules/1') jobs[0].enabled=req.postDataJSON().enabled
    else if(path==='/api/config') {
      if(req.method()==='GET')return route.fulfill({json:{config:{napcat:{token:'***'},sandbox:{enabled:false,image:'existing-sandbox',network:'none',timeout_secs:45}},providers:{}}})
      writes.push(req.postDataJSON());return route.fulfill({json:{applied:[],requires_restart:['sandbox 开发沙箱（重启生效）']}})
    }
    return route.fulfill({json:{ok:true}})
  })
  return {jobs,writes}
}

test('daily Agent plan has bounded tools, queued state, dated failures and task links',async({page})=>{
  const {jobs,writes}=await setup(page)
  await page.setViewportSize({width:1440,height:1000})
  await page.goto('/schedules')
  await expect(page.getByText('Bash 未启用，请先部署沙箱')).toBeVisible()
  await page.getByRole('button',{name:'检查 Bash 环境'}).click()
  await expect(page.getByText('Docker 或沙箱镜像不可用，请检查连接')).toBeVisible()
  await page.getByLabel('云团任务名称').fill('每日计算')
  await page.getByLabel('任务要求',{exact:true}).fill('用 Python 计算 1 到 100 的和并核对结果')
  await expect(page.getByRole('button',{name:'创建云团任务'})).toBeDisabled()
  await page.getByLabel('允许使用的工具').click()
  await page.getByText('Bash 沙箱',{exact:true}).last().click()
  await page.getByLabel('云团每日执行时间').fill('09:15')
  await page.getByRole('button',{name:'创建云团任务'}).click()
  expect(writes[0]).toMatchObject({title:'每日计算',daily_time:'09:15',allowed_tools:['sandbox_exec'],max_calls:10,timeout_secs:900})
  const row=page.locator('.agent-job')
  await expect(row).toContainText('每日计算')
  await row.getByRole('button',{name:'立即执行',exact:true}).click()
  await expect(row).toContainText('已触发，排队中')
  await expect(row.getByRole('button',{name:'立即执行',exact:true})).toBeDisabled()
  await expect(row.getByRole('link',{name:'查看完整执行过程'})).toHaveAttribute('href','/tasks?task=scheduled-1')
  Object.assign(jobs[0],{phase:'failed',error:'命令执行超时'})
  await page.getByRole('button',{name:'刷新计划'}).click()
  await expect(row).toContainText('执行失败')
  await row.getByRole('button',{name:'执行记录',exact:true}).click()
  await expect(row.locator('.run-list')).toContainText('命令执行超时')
  await row.getByRole('button',{name:'更改时间',exact:true}).click()
  await row.getByLabel('云团新的执行时间').fill('08:30')
  await row.getByRole('button',{name:'保存时间',exact:true}).click()
  await expect(row).toContainText('每日 08:30')
  await row.getByRole('button',{name:'暂停计划',exact:true}).click()
  await expect(row.getByRole('button',{name:'立即执行',exact:true})).toBeDisabled()
  await page.screenshot({path:'test-results/agent-schedules-desktop.png',fullPage:true})
  await page.setViewportSize({width:390,height:844})
  expect(await page.evaluate(()=>document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  await page.screenshot({path:'test-results/agent-schedules-mobile.png',fullPage:true})
  await page.getByRole('button',{name:'切换主题'}).click()
  await expect(page.locator('html')).toHaveClass('dark')
  await expect(page.getByLabel('云团每日执行时间')).toHaveCSS('color-scheme','dark')
  await page.screenshot({path:'test-results/agent-schedules-dark.png',fullPage:true})
})

test('unconfigured model stays explicit and Bash settings preserve unrelated configuration',async({page})=>{
  const {writes}=await setup(page,false)
  await page.goto('/schedules')
  await expect(page.getByText('尚未配置，请前往模型服务设置 Agent 执行模型')).toBeVisible()
  await expect(page.getByRole('button',{name:'创建云团任务'})).toBeDisabled()
  await page.getByRole('link',{name:'沙箱设置',exact:true}).click()
  await page.getByRole('switch',{name:'启用 Bash 沙箱'}).click()
  await page.getByRole('button',{name:'检查当前环境'}).click()
  await expect(page.getByText('Docker 或沙箱镜像不可用，请检查连接')).toBeVisible()
  await page.getByRole('button',{name:'保存运行参数'}).click()
  await expect(page.getByText(/以下配置项需重启服务后生效/)).toBeVisible()
  expect(writes[0].config.napcat.token).toBe('***')
  expect(writes[0].config.sandbox).toMatchObject({enabled:true,image:'existing-sandbox',timeout_secs:45,network:'none'})
})
