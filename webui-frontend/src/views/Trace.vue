<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">决策追踪</span>
        <span class="yt-page-sub mono">{{ items.length }} 条记录</span>
      </div>
      <n-button size="small" secondary :loading="loading" @click="reset">刷新</n-button>
    </div>

    <n-alert v-if="status?.blocked_reason" type="warning" :bordered="false">{{ status.blocked_reason }}</n-alert>
    <n-alert v-if="statusError" type="error" :bordered="false">{{ statusError }}</n-alert>
    <n-alert v-if="view === 'events' && eventsError" type="error" :bordered="false">{{ eventsError }}</n-alert>
    <div class="decision-metrics">
      <button v-for="metric in metrics" :key="metric.key" class="decision-metric" @click="showQueue(metric.key)">
        <span>{{ metric.label }}</span><strong class="mono">{{ status?.counts?.[metric.key] ?? '—' }}</strong>
      </button>
    </div>
    <n-tabs v-model:value="view" type="line" size="small"><n-tab name="events">决策记录</n-tab><n-tab name="queue">消息处理</n-tab></n-tabs>
    <div class="decision-policy" v-if="status?.policy">
      <n-tag size="small" :bordered="false">群聊 {{ status.policy.group_interval_secs }} 秒</n-tag>
      <n-tag size="small" :bordered="false">@ / 私聊即时</n-tag>
      <n-tag size="small" :bordered="false">批次并发 {{ status.policy.periodic_concurrency }}</n-tag>
      <span v-if="status.counts.retry_waiting" class="tl-chat">{{ status.counts.retry_waiting }} 条等待重试</span>
    </div>
    <div class="yt-toolbar">
      <n-select v-if="view === 'events'" v-model:value="kind" :options="kindOptions" clearable placeholder="事件类型" size="small" style="width: 190px" @update:value="reset" />
      <n-select v-else v-model:value="queueState" :options="stateOptions" clearable placeholder="处理状态" size="small" style="width: 190px" @update:value="refreshStatus" />
      <label v-if="view === 'events'" class="trace-date">
        日期 <input v-model="date" type="date" aria-label="决策日期" @change="resetEvents" />
      </label>
      <n-button v-if="view === 'events' && date" size="small" text @click="clearDate">清除日期</n-button>
      <n-input v-model:value="chat" placeholder="会话 ID" size="small" style="width: 180px" @keyup.enter="reset" />
      <n-button size="small" type="primary" secondary @click="reset">筛选</n-button>
      <span class="spacer" />
      <n-tag v-if="view === 'events' && date" size="small" :bordered="false">历史记录</n-tag>
      <n-tag v-else size="small" :type="wsConnected ? 'success' : 'default'" :bordered="false">
        {{ wsConnected ? '实时流已连接' : '实时流已断开' }}
      </n-tag>
    </div>

    <section v-if="view === 'queue'" class="queue-list">
      <n-card v-for="m in status?.messages || []" :key="m.msg_id" size="small" class="queue-card">
        <div class="queue-head"><strong>{{ m.sender_name }}</strong><span class="tl-chat mono">{{ m.sender_pid.replace(/^p_/, '') }}</span><span class="spacer" /><n-tag size="small" :bordered="false" :type="['uncertain', 'failed'].includes(m.state) ? 'warning' : 'default'">{{ stateLabel(m) }}</n-tag></div>
        <p class="queue-text">{{ m.text || '图片或表情' }}</p>
        <div class="queue-foot"><span>{{ m.chat_type === 'private' ? '私聊' : '群聊' }} {{ m.chat_id }}</span><span>#{{ m.msg_id }}</span><span>{{ fmtTs(m.ts) }}</span><span v-if="m.attempts">累计失败 {{ m.attempts }} 次</span><span v-if="m.state === 'pending' && m.retry_at > now">{{ fmtTs(m.retry_at) }} 后重试</span></div>
        <div v-if="m.error" class="queue-warning">{{ m.error }}</div>
        <div v-if="m.state === 'uncertain'" class="queue-warning">执行结果待核实</div>
      </n-card>
      <empty-state v-if="status && !status.messages.length" title="暂无匹配消息" />
    </section>
    <div v-else class="trace-workspace">
      <section class="trace-events">
        <div v-if="items.length" class="tl">
          <div v-for="e in items" :key="e.id ?? e._k" class="tl-item" :class="{ active: sel === e }" @click="sel = e">
            <div class="tl-rail"><span class="tl-dot" :style="{ background: kindColor(e.kind) }" /></div>
            <div class="tl-card" :class="{ 'tl-decision': e.kind === 'DecisionMade' }"
                 :style="e.kind === 'DecisionMade' ? { '--action-c': actionColor(e.payload?.action) } : {}">
              <div class="tl-head">
                <span class="tl-badge" :style="badgeStyle(e.kind)">{{ kindLabel(e.kind) }}</span>
                <span v-if="e.kind === 'DecisionMade' && e.payload?.action && !e.payload?.fallback" class="tl-badge"
                      :style="badgeStyleFor(e.payload?.action, actionColor(e.payload?.action))">
                  {{ actionLabel(e.payload?.action) }}
                </span>
                <span v-if="e.payload?.fallback" class="tl-badge">判断未完成</span>
                <span v-if="e.payload?.trigger" class="tl-badge">{{ triggerLabel(e.payload.trigger) }}<template v-if="e.payload.trigger === 'periodic'"> · {{ e.payload.batch_message_ids?.length || 0 }} 条</template></span>
                <time class="tl-time" :title="fmtTs(e.ts)">{{ date ? fmtTs(e.ts) : fmtAgo(e.ts) }}</time>
              </div>
              <div v-if="e.payload?.sender_name" class="tl-chat">{{ e.payload.sender_name }}</div>
              <div class="tl-text">{{ summary(e) }}</div>
              <div v-if="e.payload?.chat_id" class="tl-chat">Chat {{ e.payload.chat_id }}</div>
            </div>
          </div>
        </div>
        <n-card v-else-if="!loading && !eventsError" size="small">
          <empty-state title="暂无匹配事件" />
        </n-card>
        <n-button v-if="hasMore" block size="small" secondary style="margin-top: 10px" :loading="loading" @click="loadMore">
          加载更早记录
        </n-button>
      </section>

      <aside class="trace-detail">
        <template v-if="sel">
          <div class="detail-header">
            <n-tag size="small" :bordered="false" :color="badgeStyleObj(sel.kind)">{{ kindLabel(sel.kind) }}</n-tag>
            <span v-if="sel.ts" class="tl-chat" style="margin-top: 0">{{ fmtTs(sel.ts) }}</span>
          </div>
          <template v-if="sel.kind === 'DecisionMade'">
            <n-space size="small" style="margin-bottom: 10px">
              <n-tag v-if="!sel.payload.fallback" size="small" :bordered="false" :color="actionTagColor(sel.payload.action)">{{ actionLabel(sel.payload.action) }}</n-tag>
              <n-tag v-if="sel.payload.mood" size="small" :bordered="false">{{ sel.payload.mood }}</n-tag>
              <n-tag v-if="sel.payload.fallback" size="small" :bordered="false" type="warning">判断未完成</n-tag>
            </n-space>
            <div class="decision-source" v-if="sel.payload.anchor_msg_id">
              <strong>{{ sel.payload.sender_name || sel.payload.sender_pid }}</strong>
              <span>{{ triggerLabel(sel.payload.trigger) }}<template v-if="sel.payload.trigger === 'periodic'"> · {{ sel.payload.batch_message_ids?.length || 0 }} 条</template></span>
              <span>消息 #{{ sel.payload.anchor_msg_id }} · {{ sel.payload.chat_type === 'private' ? '私聊' : '群聊' }} {{ sel.payload.chat_id }}</span>
            </div>
            <n-alert v-if="sel.payload.reason" type="info" :bordered="false" style="margin-bottom: 10px">
              {{ sel.payload.reason }}
            </n-alert>
            <div v-if="sel.payload.policy?.activity" class="decision-policy">
              <n-tag size="small" :bordered="false">{{ heatLabel(sel.payload.policy.activity.level) }}</n-tag>
              <span class="tl-chat">近2分钟 {{ sel.payload.policy.activity.human_messages_2min }} 条 · {{ sel.payload.policy.activity.active_speakers_2min }} 人</span>
              <span v-if="sel.payload.policy.cooldown_secs" class="tl-chat">参与间隔 {{ sel.payload.policy.cooldown_secs }} 秒</span>
            </div>
            <template v-if="sel.payload.policy">
              <n-space size="small" style="margin-bottom: 10px">
                <n-tag size="small" :bordered="false">建议动作: {{ sel.payload.policy.suggested_action }}</n-tag>
                <n-tag v-if="sel.payload.policy.meme_plan" size="small" :bordered="false">计划表情: {{ memeScopeLabel(sel.payload.policy.meme_plan.scope) }} · {{ sel.payload.policy.meme_plan.category }}</n-tag>
                <n-tag v-if="sel.payload.reply_mode" size="small" :bordered="false">模式: {{ replyModeLabel(sel.payload.reply_mode) }}</n-tag>
                <n-tag v-if="sel.payload.policy.evidence_valid === false" size="small" :bordered="false" type="warning">依据未通过</n-tag>
              </n-space>
              <n-alert v-if="sel.payload.policy.notes?.length" type="warning" :bordered="false" style="margin-bottom: 10px">
                {{ sel.payload.policy.notes.join('；') }}
              </n-alert>
            </template>
          </template>
          <json-view :data="sel.payload" />
        </template>
        <empty-state v-else title="选择左侧事件查看详情" />
      </aside>
    </div>
  </div>
</template>

<script setup>
import { onMounted, onUnmounted, ref } from 'vue'
import { api } from '../api'
import { subscribeWs, wsConnected } from '../ws'
import { kindColor, kindLabel, actionColor, eventSummary, fmtAgo, fmtTs } from '../fmt'
import JsonView from '../components/JsonView.vue'
import EmptyState from '../components/EmptyState.vue'
import { mergeEventRows, messageEventKey } from '../event-feed'

/** @type {import('vue').Ref<any[]>} */
const items = ref([])
/** @type {import('vue').Ref<any>} */
const sel = ref(null)
const kind = ref(null)
const chat = ref('')
const date = ref('')
const loading = ref(false)
const hasMore = ref(false)
const statusError = ref('')
const eventsError = ref('')
const view = ref('events')
const queueState = ref(null)
/** @type {import('vue').Ref<any>} */
const status = ref(null)
const now = ref(Math.floor(Date.now() / 1000))
const metrics = [{ key: 'pending', label: '待判断' }, { key: 'deciding', label: '已领取' }, { key: 'executing', label: '执行中' }, { key: 'uncertain', label: '待核实' }, { key: 'failed', label: '已停止重试' }]
const stateOptions = metrics.map(m => ({ value: m.key, label: m.label }))
function summary(e) { return e.kind === 'DecisionMade' ? [e.payload?.fallback ? '判断未完成' : actionLabel(e.payload?.action), e.payload?.reason].filter(Boolean).join(' · ') : eventSummary(e) }
function replyModeLabel(mode) { return ({ answer: '答疑', social: '自然接话', clarify: '澄清', state_uncertainty: '说明不确定' })[mode] || mode }
function memeScopeLabel(scope) { return ({ self: '云团专属', other: '其他图库', prefer_self: '优先云团' })[scope] || scope }
function heatLabel(level) { return ({ quiet: '群聊安静', warming: '群聊渐热', lively: '群聊活跃', crowded: '群聊密集' })[level] || level }
function actionLabel(action) { return ({ reply: '回应', ignore: '保持安静', send_meme: '发表情', start_task: '派发任务', invoke_skill: '调用技能' })[action] || action }
function triggerLabel(trigger) { return ({ periodic: '群聊批次', immediate: '即时请求', task_result: '任务回聊', chat_loop: '聊天转路由' })[trigger] || trigger || '' }
function stateLabel(m) { return m.state === 'pending' && m.retry_at > now.value ? '等待重试' : metrics.find(v => v.key === m.state)?.label || m.state }
function showQueue(state) { view.value = 'queue'; queueState.value = state; refreshStatus() }
let statusLoading = false
let statusRequest = 0
async function refreshStatus() {
  const request = ++statusRequest
  statusLoading = true
  try {
    const params = new URLSearchParams()
    if (chat.value.trim()) params.set('chat_id', chat.value.trim())
    if (queueState.value) params.set('state', queueState.value)
    const { data } = await api.get(`/decisions/status?${params}`)
    if (request !== statusRequest) return
    status.value = data
    now.value = Math.floor(Date.now() / 1000)
    statusError.value = ''
  } catch (e) { if (request === statusRequest) statusError.value = e.response?.data?.error || '读取消息处理状态失败' }
  finally { if (request === statusRequest) statusLoading = false }
}

const kindOptions = [
  'MessageReceived', 'DecisionMade', 'BubbleSent', 'ReplyInterrupted',
  'ConsolidationDone', 'MemoryWritten', 'MoodChanged', 'ConfigReloaded',
].map((k) => ({ label: kindLabel(k), value: k }))

function badgeStyle(k) {
  return badgeStyleFor(k, kindColor(k))
}
function badgeStyleFor(_k, c) {
  return { color: c, background: `color-mix(in srgb, ${c} 10%, transparent)`, border: `1px solid color-mix(in srgb, ${c} 24%, transparent)` }
}
function badgeStyleObj(k) {
  const c = kindColor(k)
  return { color: `color-mix(in srgb, ${c} 10%, transparent)`, textColor: c }
}
function actionTagColor(a) {
  const c = actionColor(a)
  return { color: `color-mix(in srgb, ${c} 10%, transparent)`, textColor: c }
}

let eventsRequest = null
let liveDuringLoad = []
async function fetchPage(beforeId = null) {
  eventsRequest?.abort()
  const request = new AbortController()
  const arrivals = []
  liveDuringLoad = arrivals
  eventsRequest = request
  loading.value = true
  eventsError.value = ''
  try {
    const params = new URLSearchParams({ limit: '50' })
    if (kind.value) params.set('kind', kind.value)
    if (chat.value.trim()) params.set('chat_id', chat.value.trim())
    if (beforeId) params.set('before_id', String(beforeId))
    if (date.value) {
      const start = new Date(`${date.value}T00:00:00`)
      const end = new Date(start)
      end.setDate(end.getDate() + 1)
      params.set('from_ts', String(start.getTime() / 1000))
      params.set('until_ts', String(end.getTime() / 1000))
    }
    const { data } = await api.get(`/events?${params}`, { signal: request.signal })
    if (request.signal.aborted) return
    if (beforeId) {
      items.value = mergeEventRows([...items.value, ...data.events])
    } else {
      items.value = mergeEventRows([...arrivals, ...data.events])
      sel.value = items.value[0] || null
    }
    hasMore.value = data.has_more ?? (data.events.length === 50)
  } catch (error) {
    if (!request.signal.aborted) eventsError.value = error.response?.data?.error || '读取决策记录失败'
  } finally {
    if (eventsRequest === request) {
      eventsRequest = null
      loading.value = false
    }
  }
}

function resetEvents() {
  items.value = []
  sel.value = null
  hasMore.value = false
  return fetchPage()
}
function clearDate() {
  date.value = ''
  return resetEvents()
}
function reset() {
  return Promise.all([refreshStatus(), resetEvents()])
}
async function loadMore() {
  if (loading.value || !hasMore.value) return
  const oldest = items.value[items.value.length - 1]
  if (!oldest?.id) return
  await fetchPage(oldest.id)
}

let off = null
let poll = null
onMounted(() => {
  void reset()
  poll = setInterval(() => { if (!document.hidden && !statusLoading) refreshStatus() }, 10000)
  off = subscribeWs((ev) => {
    if (date.value || !ev?.kind) return
    if (kind.value && ev.kind !== kind.value) return
    if (chat.value && ev.chat_id !== chat.value && ev.payload?.chat_id !== chat.value) return
    const key = messageEventKey(ev)
    if (key != null && items.value.some(item => messageEventKey(item) === key)) return
    const item = { _k: Math.random(), kind: ev.kind, payload: ev.payload ?? ev, ts: ev.ts ?? Math.floor(Date.now() / 1000) }
    if (eventsRequest) { liveDuringLoad.unshift(item); liveDuringLoad.length = Math.min(liveDuringLoad.length, 300) }
    items.value = mergeEventRows([item, ...items.value], 300)
  })
})
onUnmounted(() => { statusRequest++; eventsRequest?.abort(); off?.(); if (poll) clearInterval(poll) })
</script>

<style scoped>
.decision-metrics { display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap: 12px; }
.decision-metric { border: 1px solid var(--yt-card-border); border-radius: 12px; background: var(--yt-surface); padding: 16px; text-align: left; color: var(--yt-ink-2); font: inherit; cursor: pointer; display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.decision-metric strong { color: var(--yt-ink-1); font-size: 23px; }
.decision-metric:hover { border-color: var(--yt-primary); }
.decision-policy, .queue-head, .queue-foot { display: flex; align-items: center; flex-wrap: wrap; gap: 10px; }
.decision-policy { margin: 2px 0 12px; }
.trace-date { display: inline-flex; align-items: center; gap: 6px; color: var(--yt-ink-2); font-size: 13px; }
.trace-date input { min-height: 28px; padding: 2px 7px; border: 1px solid var(--yt-card-border); border-radius: 6px; background: var(--yt-surface); color: var(--yt-ink-1); font: inherit; }
.queue-list { display: grid; gap: 12px; }
.queue-text { white-space: pre-wrap; overflow-wrap: anywhere; margin: 12px 0; color: var(--yt-ink-1); }
.queue-foot { color: var(--yt-ink-3); font-size: 12px; }
.queue-warning { margin-top: 10px; color: var(--yt-warn, #b7791f); font-size: 12px; }
.decision-source { display: flex; flex-direction: column; gap: 6px; margin: 0 0 14px; font-size: 12px; color: var(--yt-ink-3); }
.decision-source strong { font-size: 15px; color: var(--yt-ink-1); }
@media (max-width: 620px) { .decision-metrics { grid-template-columns: repeat(2, minmax(0, 1fr)); } .decision-metric { padding: 12px; } }

.trace-workspace {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 440px;
  gap: 16px;
  align-items: start;
}
.trace-events { min-width: 0; }
.trace-detail {
  min-width: 0;
  background: var(--yt-surface);
  border: 1px solid var(--yt-card-border);
  border-radius: 12px;
  padding: 18px;
  box-shadow: var(--yt-card-shadow);
}
.detail-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}
@media (max-width: 1080px) {
  .trace-workspace { grid-template-columns: 1fr; }
}
</style>
