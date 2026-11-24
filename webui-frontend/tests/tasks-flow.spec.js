import { expect, test } from '@playwright/test'

test('task controls expose waiting, delivery and errors without repeating a rejected action', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('yt_token', 'test-only'))
  await page.routeWebSocket('**/ws**', () => {})
  let phase = 'executing'
  let goal = '整理资料'
  const writes = []
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url())
    if (url.pathname === '/api/tasks') return route.fulfill({ json: { tasks: [{ task_id: 't1', goal, state: 'running', used_calls: 2, budget_max_calls: 8 }] } })
    if (url.pathname.endsWith('/events')) return route.fulfill({ json: { events: [] } })
    if (url.pathname.endsWith('/runtime')) return route.fulfill({ json: { runtime: { phase, delivery_state: 'unknown', stop_reason: '发送结果待核实' }, brief: { revision: 2, goal, request: { msg_id: 10, text: '比较A和B', ts: 100 }, working_context: [{ context_id: 10, conditions: [{ key: '显卡', quote: '4090', source_msg_id: 11 }, { key: '环境', quote: '只能本地运行', source_msg_id: 11 }] }], changes: [{ text: '增加来源列', source: { msg_id: 12 } }] }, brief_versions: [{ revision: 2, event_kind: 'amend', source_msg_id: 12 }], invocations: [{ id: 'i1', tool: 'search', state: 'started', status: 'unknown', call_id: 'native_42', revision: 2, protocol: 'native', elapsed_ms: 120000, error: { kind: 'timeout', message: '远端执行结果需核实' }, started_at: 100 }] } })
    if (url.pathname.endsWith('/control')) {
      const body = route.request().postDataJSON()
      writes.push(body.action)
      if (body.action === 'cancel') return route.fulfill({ status: 400, json: { error: '外部动作结果未确认' } })
      phase = body.action === 'pause' ? 'paused' : 'executing'
      return route.fulfill({ json: { ok: true } })
    }
    return route.fulfill({ json: {} })
  })
  await page.goto('/tasks')
  await expect(page.getByText('送达待核实', { exact: true })).toBeVisible()
  await expect(page.getByText('任务说明', { exact: true })).toBeVisible()
  await expect(page.getByText('4090', { exact: true })).toBeVisible()
  await expect(page.getByText('只能本地运行', { exact: true })).toBeVisible()
  await page.getByText('补充要求（1）', { exact: true }).click()
  await expect(page.getByText('增加来源列', { exact: true })).toBeVisible()
  await page.getByText('工具执行记录', { exact: true }).click()
  await expect(page.getByText(/search · 结果待核实/)).toBeVisible()
  await expect(page.getByText(/native \/ native_42 \/ v2/)).toBeVisible()
  await expect(page.getByText('远端执行结果需核实', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: '暂停', exact: true }).click()
  await expect(page.getByText('已暂停', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: '继续', exact: true }).click()
  await expect(page.getByText('执行中', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: '取消任务', exact: true }).click()
  await expect(page.getByText('外部动作结果未确认', { exact: true })).toBeVisible()
  expect(writes).toEqual(['pause', 'resume', 'cancel'])
  goal = '整理资料并补来源'
  await page.getByRole('button', { name: '刷新', exact: true }).click()
  await expect(page.locator('.task-brief').getByText(goal, { exact: true })).toBeVisible()
})

test('creates a daily tool task from the navigation and manages its result and time', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('yt_token', 'test-only'))
  await page.routeWebSocket('**/ws**', () => {})
  const tasks = []
  const writes = []
  const option = { tool_name: 'quark-drive.search', label: '搜索夸克网盘', input: 'keyword', unavailable: null }
  await page.route('**/api/**', async route => {
    const request = route.request()
    const path = new URL(request.url()).pathname
    if (path === '/api/schedules') {
      if (request.method() === 'GET') return route.fulfill({ json: { tasks, options: [option] } })
      const body = request.postDataJSON()
      writes.push(body)
      const [local_hour, local_minute] = body.daily_time.split(':').map(Number)
      tasks.push({ id: 1, title: body.title, tool_name: body.tool_name, resource_key: body.input, local_hour, local_minute,
        enabled: true, running: false, next_run_at: 1810000000, last_run_at: null, last_status: 'pending' })
    } else if (path === '/api/schedules/1/run') {
      Object.assign(tasks[0], { last_run_at: 1800000000, last_status: 'accessible', last_summary: '找到 2 个文件' })
    } else if (path === '/api/schedules/1/time') {
      const [local_hour, local_minute] = request.postDataJSON().daily_time.split(':').map(Number)
      Object.assign(tasks[0], { local_hour, local_minute })
    } else if (path === '/api/schedules/1') {
      if (request.method() === 'DELETE') tasks.splice(0)
      else tasks[0].enabled = request.postDataJSON().enabled
    }
    return route.fulfill({ json: { ok: true } })
  })
  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto('/tasks')
  await page.getByRole('button', { name: '切换导航栏' }).click()
  await page.getByText('定时任务', { exact: true }).click()
  await expect(page).toHaveURL(/\/schedules$/)
  await page.getByText('固定工具', { exact: true }).click()
  await page.getByLabel('执行工具').click()
  await page.getByText('搜索夸克网盘', { exact: true }).click()
  await page.getByLabel('搜索关键词').fill('工作记录')
  await page.getByLabel('任务名称').fill('每日找资料')
  await page.getByLabel('每日执行时间', { exact: true }).fill('18:15')
  await page.getByRole('button', { name: '创建定时任务' }).click()
  const row = page.locator('.schedule-row')
  await expect(row).toContainText('每日找资料')
  await expect(row).toContainText('工作记录')
  expect(writes).toEqual([{ tool_name: option.tool_name, title: '每日找资料', daily_time: '18:15', input: '工作记录' }])
  await row.getByRole('button', { name: '更改时间' }).click()
  await row.getByLabel('新的每日执行时间').fill('08:30')
  await row.getByRole('button', { name: '保存时间' }).click()
  await expect(row).toContainText('08:30')
  await row.getByRole('button', { name: '立即执行' }).click()
  await expect(row).toContainText('执行成功')
  await row.getByText('查看最近结果').click()
  await expect(row.getByText('找到 2 个文件')).toBeVisible()
  await row.getByRole('button', { name: '暂停' }).click()
  await expect(row.getByRole('button', { name: '立即执行' })).toBeDisabled()
  await row.getByRole('button', { name: '删除' }).click()
  await page.getByRole('button', { name: '删除任务', exact: true }).click()
  await expect(page.getByText('暂无定时任务')).toBeVisible()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
})
