import { expect, test } from '@playwright/test'

const now = Date.now()
const entries = [
  { id: 102, ts: now, level: 'ERROR', target: 'yuantuan_core::llm', message: '模型请求失败，服务返回 HTTP 429', file: 'crates/core/src/llm.rs', line: 754, fields: { chat_id: '42', source_msg_id: '88', error: 'HTTP 429\nrate limit exceeded\nrequest id: example' } },
  { id: 101, ts: now - 2000, level: 'INFO', target: 'yuantuan_core::supervisor', message: 'supervisor 启动组件', fields: { component: 'pipeline' } },
]
async function setup(page) {
  const requests = []
  await page.addInitScript(() => localStorage.setItem('yt_token', 'test-only'))
  await page.routeWebSocket('**/ws**', () => {})
  await page.route('**/api/**', route => {
    const url = new URL(route.request().url())
    if (url.pathname === '/api/logs') {
      requests.push(url.searchParams)
      const rows = url.searchParams.get('search') ? entries.filter(e => e.message.includes(url.searchParams.get('search'))) : entries
      return route.fulfill({ json: { entries: rows, latest_id: 102, oldest_id: 101, targets: [...new Set(entries.map(e => e.target))], has_more: false, storage: { capacity: 2000, persisting: true, minimum_level: 'INFO', dropped_writes: 0, disk_errors: 0 } } })
    }
    if (url.pathname === '/api/runtime/status') return route.fulfill({ json: {
      process: { pid: 1234, memory_bytes: 192 * 1024 * 1024, cpu_percent: 2.4, children: [{ pid: 5678, name: 'node', memory_bytes: 24 * 1024 * 1024, run_time_secs: 65 }] },
      uptime_secs: 7325, total_restarts: 2, bash_registered: false,
      components: [{ name: 'pipeline', state: 'running', restarts: 0, changed_at: now / 1000 }, { name: 'task_runner', state: 'backoff', restarts: 2, changed_at: now / 1000, reason: 'component_exited' }],
      platforms: [{ id: 'onebot', state: 'online' }],
    } })
    return route.fulfill({ json: {} })
  })
  return requests
}

test('runtime console displays errors, filters logs and inspects real component states', async ({ page }) => {
  const requests = await setup(page)
  await page.setViewportSize({ width: 1512, height: 980 })
  await page.goto('/logs')
  await expect(page.getByRole('heading', { name: '模型请求失败，服务返回 HTTP 429' })).toBeVisible()
  await expect(page.getByLabel('日志详情')).toContainText('rate limit exceeded')
  await expect(page.getByLabel('日志详情')).toContainText('crates/core/src/llm.rs:754')
  await page.locator('.log-row').filter({ hasText: 'supervisor 启动组件' }).click()
  await expect(page.getByLabel('日志详情')).toContainText('pipeline')
  await expect(page.getByLabel('日志详情')).not.toContainText('rate limit exceeded')
  await page.getByRole('button', { name: '暂停更新' }).click()
  await expect(page.getByText('已暂停更新')).toBeVisible()
  await page.getByLabel('搜索日志').fill('429')
  await page.getByRole('button', { name: '筛选', exact: true }).click()
  await expect(page.locator('.log-row')).toHaveCount(1)
  expect(requests.at(-1).get('search')).toBe('429')
  await page.getByLabel('日志日期').fill('2026-11-23')
  await expect.poll(() => requests.at(-1).has('from_ts')).toBe(true)
  expect(Number(requests.at(-1).get('until_ts'))).toBeGreaterThan(Number(requests.at(-1).get('from_ts')))
  await page.getByRole('tab', { name: '进程与组件' }).click()
  await expect(page.getByText('等待重启', { exact: true })).toBeVisible()
  await expect(page.getByText('OneBot / NapCat')).toBeVisible()
  await expect(page.getByText('已连接', { exact: true })).toBeVisible()
  await expect(page.locator('.child-processes')).toContainText('5678')
})

test('console fits mobile and preserves readable error detail', async ({ page }) => {
  await setup(page)
  await page.setViewportSize({ width: 390, height: 844 })
  await page.goto('/logs')
  await expect(page.locator('.log-row')).toHaveCount(2)
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)).toBe(true)
  await expect(page.getByLabel('日志详情')).toContainText('HTTP 429')
  await page.locator('.log-row').filter({ hasText: 'supervisor 启动组件' }).click()
  await expect(page.getByLabel('日志详情')).toContainText('pipeline')
  await page.getByRole('tab', { name: '进程与组件' }).click()
  await expect(page.getByText('等待重启', { exact: true })).toBeVisible()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)).toBe(true)
})

test('log panels follow the application theme and keep messages readable', async ({ page }) => {
  await setup(page)
  await page.setViewportSize({ width: 1512, height: 980 })
  await page.goto('/logs')
  await expect(page.locator('.log-row')).toHaveCount(2)
  const surfaces = () => page.evaluate(() => {
    const background = selector => getComputedStyle(document.querySelector(selector)).backgroundColor
    return { list: background('.log-list'), detail: background('.log-detail'), toolbar: background('.log-filters') }
  })
  const light = await surfaces()
  expect(light.list).toBe(light.detail)
  expect(light.list).toBe(light.toolbar)
  expect(light.list).toBe('rgb(255, 255, 255)')
  expect(await page.locator('.log-content').first().evaluate(el => Number.parseFloat(getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(13)
  await page.getByRole('button', { name: '切换主题' }).click()
  await expect(page.locator('html')).toHaveClass('dark')
  const dark = await surfaces()
  expect(dark.list).not.toBe(light.list)
  expect(dark.list).toBe(dark.detail)
  expect(dark.list).toBe(dark.toolbar)
  await expect(page.getByLabel('日志详情')).toContainText('rate limit exceeded')
})

test('log read failures stay visible', async ({ page }) => {
  await setup(page)
  await page.route('**/api/logs?**', route => route.fulfill({ status: 500, json: { error: '日志读取失败' } }))
  await page.goto('/logs')
  await expect(page.getByText('日志读取失败', { exact: true })).toBeVisible()
})

test('initial read failure recovers with history pagination', async ({ page }) => {
  await setup(page)
  let reads = 0
  await page.route('**/api/logs?**', route => {
    if (++reads === 1) return route.fulfill({ status: 500, json: { error: '日志读取失败' } })
    return route.fulfill({ json: { entries, latest_id: 102, has_more: true, targets: [], storage: { capacity: 2000, minimum_level: 'INFO' } } })
  })
  await page.goto('/logs')
  await expect(page.getByText('日志读取失败', { exact: true })).toBeVisible()
  await expect(page.locator('.log-row')).toHaveCount(2)
  await expect(page.getByRole('button', { name: '加载更早记录' })).toBeVisible()
  await expect(page.getByRole('heading', { name: '模型请求失败，服务返回 HTTP 429' })).toBeVisible()
})
