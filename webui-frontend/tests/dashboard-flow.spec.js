import { expect, test } from '@playwright/test'
import path from 'node:path'
import fs from 'node:fs'

function received(msgId, text = '1') {
  return { kind: 'MessageReceived', msg_id: msgId, chat_id: '42', chat_type: 'group',
    sender_pid: 'p_2001', text, ts: Math.floor(Date.now() / 1000) }
}
function stored(event, id = event.msg_id) {
  return { id, kind: event.kind, payload: event, ts: event.ts }
}
async function eventFixture(page, waitForSnapshot = null) {
  const first = received(11)
  const state = { socket: null, inbound: 1, reads: 0, eventsRequested: false }
  await page.addInitScript(() => localStorage.setItem('yt_token', 'test-only'))
  await page.routeWebSocket('**/ws**', socket => { state.socket = socket })
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url())
    if (url.pathname === '/api/dashboard') {
      state.reads++
      return route.fulfill({ json: { messages_in_today: state.inbound, messages_out_today: 0,
        decision_calls_today: 0, active_tasks: 0, mood: 'calm' } })
    }
    if (url.pathname === '/api/events') {
      state.eventsRequested = true
      if (waitForSnapshot) await waitForSnapshot
      return route.fulfill({ json: { events: [stored(first)] } })
    }
    return route.fulfill({ json: {} })
  })
  return { state, first }
}

test('HTTP and live copies appear once while separate identical messages remain visible', async ({ page }) => {
  const { state, first } = await eventFixture(page)
  await page.goto('/')
  const messages = page.locator('.mini-event').filter({ hasText: '收到消息' })
  await expect(messages).toHaveCount(1)
  state.socket.send(JSON.stringify(first))
  state.inbound = 2
  state.socket.send(JSON.stringify({ ...first, msg_id: 12 }))
  state.socket.send(JSON.stringify({ kind: 'MoodChanged', text: '实时推送已处理' }))
  await expect(page.locator('.mini-text').filter({ hasText: '实时推送已处理' })).toBeVisible()
  await expect(messages).toHaveCount(2)
  await expect(messages.locator('.mini-text')).toHaveText(['1', '1'])
  const count = page.locator('.stat-card').filter({ hasText: '今日接收消息' }).locator('.stat-num')
  await expect(count).toHaveText('2')
  state.socket.send(JSON.stringify({ ...first, msg_id: 12 }))
  state.socket.send(JSON.stringify({ kind: 'MoodChanged', text: '重复推送已处理' }))
  await expect(page.locator('.mini-text').filter({ hasText: '重复推送已处理' })).toBeVisible()
  await expect(messages).toHaveCount(2)
  await expect(count).toHaveText('2')
  await expect(page.locator('.hourbars-svg title').filter({ hasText: '收到 2 / 发送 0' })).toHaveCount(1)
})

test('live messages survive an in-flight snapshot without duplicate rows or counters', async ({ page }) => {
  let release
  const gate = new Promise(resolve => { release = resolve })
  const { state, first } = await eventFixture(page, gate)
  await page.goto('/')
  await expect.poll(() => state.eventsRequested && Boolean(state.socket)).toBe(true)
  const before = state.reads
  state.socket.send(JSON.stringify(first))
  state.inbound = 2
  state.socket.send(JSON.stringify(received(12, '加载期间的新消息')))
  // The coalesced counter request confirms both ordered socket frames were handled.
  await expect.poll(() => state.reads).toBeGreaterThan(before)
  release()
  const messages = page.locator('.mini-event').filter({ hasText: '收到消息' })
  await expect(messages).toHaveCount(2)
  await expect(messages.locator('.mini-text')).toHaveText(['加载期间的新消息', '1'])
  await expect(page.locator('.stat-card').filter({ hasText: '今日接收消息' }).locator('.stat-num')).toHaveText('2')
  await expect(page.locator('.hourbars-svg title').filter({ hasText: '收到 2 / 发送 0' })).toHaveCount(1)
})

test('today and cumulative model consumption remain distinct on desktop and mobile', async ({ page }) => {
  const artifacts = path.resolve('../data/artifacts/chat-fix-20261016')
  fs.mkdirSync(artifacts, { recursive: true })
  await page.addInitScript(() => localStorage.setItem('yt_token', 'test-only'))
  await page.routeWebSocket('**/ws**', () => {})
  await page.route('**/api/**', route => {
    const pathname = new URL(route.request().url()).pathname
    return route.fulfill({ json: pathname === '/api/dashboard' ? {
      uptime_secs: 405, adapter_connected: true, mood: 'calm',
      messages_in_today: 12, messages_out_today: 3, decision_calls_today: 5, active_tasks: 0,
      tokens_today: 300, tokens_in_today: 200, tokens_out_today: 100, llm_calls_today: 3,
      tokens_total: 900, tokens_in_total: 600, tokens_out_total: 300, llm_calls_total: 9,
      mem_rss_bytes: 33554432, mem_total_bytes: 1073741824, cpu_percent: 1.5,
    } : pathname === '/api/events' ? { events: [] } : {} })
  })
  await page.setViewportSize({ width: 1440, height: 1100 })
  await page.goto('/')
  const today = page.locator('.resource-block').filter({ hasText: '今日 Token 总量' })
  const total = page.locator('.resource-block').filter({ hasText: '累计 Token 总量' })
  await expect(today.locator('.resource-value')).toHaveText('300')
  await expect(total.locator('.resource-value')).toHaveText('900')
  await expect(total.locator('.resource-cap')).toHaveText('9 次调用')
  await expect(total.locator('.token-item .v')).toHaveText(['600', '300'])
  await page.screenshot({ path: path.join(artifacts, 'dashboard-desktop.png'), fullPage: true })
  await page.setViewportSize({ width: 390, height: 844 })
  await expect(total).toBeVisible()
  expect(await page.locator('.resource-card').evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true)
  await page.screenshot({ path: path.join(artifacts, 'dashboard-mobile.png'), fullPage: true })
})
