<template>
  <n-card size="small" title="调用历史">
    <p class="history-description">最近 50 条已登记的 Skill 和任务工具调用。</p>
    <div class="yt-toolbar history-toolbar">
      <label>日期 <input v-model="date" type="date" aria-label="插件调用日期" /></label>
      <n-button v-if="date" size="small" text @click="date = ''">清除日期</n-button>
      <n-button size="small" secondary :loading="loading" @click="load">刷新记录</n-button>
    </div>
    <n-alert v-if="error" type="error" :bordered="false">{{ error }}</n-alert>
    <n-spin :show="loading">
      <ul v-if="items.length" class="history-list">
        <li v-for="entry in items" :key="`${entry.kind}:${entry.id}`">
          <div class="history-entry">
            <code>{{ entry.name }}</code>
            <n-tag size="small" :type="status(entry.status).tone" :bordered="false">{{ status(entry.status).label }}</n-tag>
          </div>
          <div class="history-meta">
            <time>{{ fmtTs(entry.started_at) }}</time>
            <span>{{ entry.kind === 'skill' ? 'Skill' : '任务工具' }}</span>
            <span v-if="entry.chat_id">{{ chatLabel(entry) }}</span>
            <span v-if="entry.anchor_id">来源消息 #{{ entry.anchor_id }}</span>
            <span v-if="entry.task_id">任务 {{ entry.task_id }}</span>
            <span v-if="entry.elapsed_ms != null">{{ pluginDuration(entry.elapsed_ms) }}</span>
          </div>
        </li>
      </ul>
      <EmptyState v-else-if="!loading && !error" title="暂无匹配的调用记录" />
    </n-spin>
  </n-card>
</template>

<script setup>
import { onUnmounted, ref, watch } from 'vue'
import { api } from '../api'
import { fmtTs } from '../fmt'
import { pluginDuration } from '../plugin-ui'
import EmptyState from './EmptyState.vue'

const props = defineProps({ name: { type: String, required: true } })
const date = ref('')
const items = ref([])
const error = ref('')
const loading = ref(false)
let request
const statuses = {
  started: { label: '已登记', tone: 'info' },
  succeeded: { label: '成功', tone: 'success' },
  failed: { label: '执行失败', tone: 'error' },
  rejected: { label: '调用被拒绝', tone: 'warning' },
  cancelled: { label: '已取消', tone: 'default' },
  unknown: { label: '结果待核实', tone: 'warning' },
}
function status(value) { return statuses[value] || { label: '状态待核实', tone: 'default' } }
function chatLabel(entry) { return [({group:'群聊',private:'私聊'})[entry.chat_type] || '会话', entry.chat_id].join(' ') }
async function load() {
  request?.abort()
  const active = new AbortController()
  request = active
  items.value = []
  error.value = ''
  loading.value = true
  try {
    const params = new URLSearchParams()
    if (date.value) {
      const start = new Date(`${date.value}T00:00:00`)
      const end = new Date(start)
      end.setDate(end.getDate() + 1)
      params.set('from_ts', String(start.getTime() / 1000))
      params.set('until_ts', String(end.getTime() / 1000))
    }
    const { data } = await api.get(`/plugins/${encodeURIComponent(props.name)}/history?${params}`, { signal: active.signal })
    if (!active.signal.aborted) items.value = data.items || []
  } catch (failure) {
    if (!active.signal.aborted) error.value = failure.response?.data?.error || '读取插件调用记录失败'
  } finally {
    if (request === active) loading.value = false
  }
}
watch([() => props.name, date], load, { immediate: true })
onUnmounted(() => request?.abort())
</script>

<style scoped>
.history-description { margin: 0 0 10px; color: var(--yt-ink-3); font-size: 13px; }
.history-toolbar { margin-bottom: 12px; }
.history-toolbar label { display: inline-flex; align-items: center; gap: 8px; color: var(--yt-ink-2); }
.history-toolbar input { border: 1px solid var(--yt-card-border); border-radius: 6px; padding: 5px; background: var(--yt-surface); color: var(--yt-ink-1); }
.history-list { list-style: none; margin: 0; padding: 0; }
.history-list li { padding: 12px 0; border-bottom: 1px solid var(--yt-card-border); }
.history-list li:last-child { border-bottom: 0; }
.history-entry, .history-meta { display: flex; gap: 8px 12px; align-items: center; flex-wrap: wrap; overflow-wrap: anywhere; }
.history-meta { margin-top: 6px; color: var(--yt-ink-3); font-size: 12px; }
</style>
