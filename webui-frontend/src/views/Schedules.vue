<template>
  <main class="yt-page schedules-page">
    <header class="yt-page-head">
      <div class="yt-page-head-left">
        <h1 class="yt-page-title">定时任务</h1>
        <span class="yt-page-sub">北京时间</span>
      </div>
      <n-button size="small" secondary :loading="loading" @click="load">刷新</n-button>
    </header>
    <n-alert v-if="loadError" type="error" :bordered="false">{{ loadError }}</n-alert>

    <div class="schedules-grid">
      <n-card title="新建任务" size="small" class="schedule-editor">
        <n-form label-placement="top" :show-feedback="false" :disabled="creating" @submit.prevent="createTask">
            <n-form-item label="执行工具">
              <n-select v-model:value="draft.tool" :options="toolOptions" placeholder="选择只读工具" aria-label="执行工具" />
            </n-form-item>
            <p v-if="options.length && !options.some(item => !item.unavailable)" class="field-note">在插件页启用工具，并检查是否允许定时任务会话。</p>
            <n-form-item v-if="selected?.input" :label="inputLabel">
              <n-input v-model:value="draft.input" :maxlength="selected.input === 'url' ? 1024 : 50" :input-props="{'aria-label':inputLabel}" />
            </n-form-item>
            <n-form-item label="任务名称（可选）">
              <n-input v-model:value="draft.title" maxlength="100" :input-props="{'aria-label':'任务名称'}" />
            </n-form-item>
            <n-form-item label="每日执行时间">
              <input v-model="draft.time" class="time-input" type="time" aria-label="每日执行时间" required :disabled="creating" />
            </n-form-item>
            <n-alert v-if="createError" type="error" :bordered="false">{{ createError }}</n-alert>
            <div class="create-actions">
              <n-button type="primary" attr-type="submit" :loading="creating" :disabled="!canCreate">创建定时任务</n-button>
            </div>
        </n-form>
      </n-card>

      <n-card size="small" class="schedule-list">
        <template #header>已登记任务</template>
        <template #header-extra><span class="yt-page-sub">{{ tasks.length }} 个</span></template>
        <n-alert v-if="actionError" type="error" :bordered="false">{{ actionError }}</n-alert>
        <n-skeleton v-if="loading && !loaded" text :repeat="5" />
        <EmptyState v-else-if="!tasks.length && !loadError" title="暂无定时任务" />
        <article v-for="task in visibleTasks" :key="task.id" class="schedule-row">
          <div class="schedule-row-head">
            <div class="schedule-name">
              <strong>{{ task.title }}</strong>
              <span>{{ options.find(option => option.tool_name === task.tool_name)?.label || task.tool_name }}</span>
              <span v-if="task.resource_key">{{ resourceLabel(task) }}</span>
            </div>
            <n-tag size="small" :type="task.running ? 'info' : task.enabled ? 'success' : 'default'" :bordered="false">
              {{ task.running ? '执行中' : task.enabled ? '已启用' : '已暂停' }}
            </n-tag>
          </div>
          <dl class="schedule-times">
            <div><dt>每日</dt><dd>{{ taskTime(task) }}</dd></div>
            <div v-if="task.enabled"><dt>下次执行</dt><dd>{{ formatTime(task.next_run_at) }}</dd></div>
            <div v-if="task.last_run_at"><dt>最近一次</dt><dd>{{ formatTime(task.last_run_at) }} · {{ resultLabel(task.last_status) }}</dd></div>
          </dl>
          <details v-if="task.last_summary" class="schedule-result">
            <summary>查看最近结果</summary>
            <p>{{ task.last_summary }}</p>
          </details>
          <div v-if="editing?.id === task.id" class="schedule-actions">
            <input v-model="editing.time" class="time-input" type="time" aria-label="新的每日执行时间" />
            <n-button size="small" type="primary" :disabled="busy.has(task.id) || task.running || !validTime(editing.time)" @click="saveTime(task)">保存时间</n-button>
            <n-button size="small" text @click="editing = null">取消</n-button>
          </div>
          <div v-else class="schedule-actions">
            <n-button size="small" secondary :loading="busy.has(task.id)" :disabled="busy.has(task.id) || task.running || !task.enabled" @click="runNow(task)">立即执行</n-button>
            <n-button size="small" text :disabled="busy.has(task.id) || task.running" @click="editing = {id: task.id, time: taskTime(task)}">更改时间</n-button>
            <n-button size="small" text :disabled="busy.has(task.id)" @click="toggle(task)">{{ task.enabled ? '暂停' : '启用' }}</n-button>
            <n-popconfirm positive-text="删除任务" negative-text="取消" @positive-click="remove(task)">
              <template #trigger><n-button size="small" text type="error" :disabled="busy.has(task.id) || task.running">删除</n-button></template>
              删除定时任务「{{ task.title }}」？
            </n-popconfirm>
          </div>
        </article>
        <n-pagination v-if="tasks.length > pageSize" v-model:page="page" :page-size="pageSize" :item-count="tasks.length" :page-slot="5" />
      </n-card>
    </div>
  </main>
</template>

<script setup>
import { computed, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import { useMessage } from 'naive-ui'
import { api } from '../api'
import EmptyState from '../components/EmptyState.vue'

const tasks = ref([])
const options = ref([])
const message = useMessage()
const page = ref(1)
const pageSize = 10
const visibleTasks = computed(() => tasks.value.slice((page.value - 1) * pageSize, page.value * pageSize))
const loading = ref(false)
const loaded = ref(false)
const creating = ref(false)
const busy = reactive(new Set())
const draft = reactive({ tool: null, input: '', title: '', time: '09:00' })
const editing = ref(null)
const loadError = ref('')
const createError = ref('')
const actionError = ref('')
watch(() => draft.tool, () => { draft.input = ''; createError.value = '' })
const selected = computed(() => options.value.find(item => item.tool_name === draft.tool))
const toolOptions = computed(() => options.value.map(item => ({
  value: item.tool_name,
  label: item.unavailable ? `${item.label}（${item.unavailable}）` : item.label,
  disabled: !!item.unavailable,
})))
const inputLabel = computed(() => selected.value?.input === 'url' ? '夸克分享链接' : '搜索关键词')
const validTime = value => /^(?:[01]\d|2[0-3]):[0-5]\d$/.test(value)
const canCreate = computed(() => !!selected.value && !selected.value.unavailable && validTime(draft.time)
  && (!selected.value.input || !!draft.input.trim()) && !creating.value)
const taskTime = task => `${String(task.local_hour).padStart(2, '0')}:${String(task.local_minute).padStart(2, '0')}`
const resourceLabel = task => task.resource_key.startsWith('https://') ? task.resource_key.split('?')[0] : task.resource_key
const formatTime = timestamp => timestamp ? new Date(timestamp * 1000).toLocaleString('zh-CN', { timeZone: 'Asia/Shanghai', hour12: false }) : '—'
const resultLabel = status => ({ accessible: '执行成功', check_failed: '执行失败', checking: '等待恢复' })[status] || '等待执行'
let request
let timer
let disposed = false

async function load() {
  if (disposed) return
  request?.abort()
  const current = new AbortController()
  request = current
  loading.value = true
  try {
    const { data } = await api.get('/schedules', { signal: current.signal })
    if (current.signal.aborted) return
    tasks.value = data.tasks || []
    options.value = data.options || []
    page.value = Math.min(page.value, Math.max(1, Math.ceil(tasks.value.length / pageSize)))
    loaded.value = true
    loadError.value = ''
  } catch (error) {
    if (!current.signal.aborted) loadError.value = error.response?.data?.error || '无法读取定时任务'
  } finally {
    if (request === current) loading.value = false
  }
}
async function createTask() {
  if (!canCreate.value) return
  creating.value = true
  createError.value = ''
  try {
    await api.post('/schedules', { tool_name: draft.tool, title: draft.title, daily_time: draft.time,
      input: selected.value.input ? draft.input.trim() : '' })
    draft.tool = null
    draft.input = ''
    draft.title = ''
    page.value = 1
    await load()
    if (!disposed) message.success('定时任务已创建')
  } catch (error) {
    createError.value = error.response?.data?.error || '创建任务失败'
  } finally {
    creating.value = false
  }
}
async function mutate(task, operation) {
  if (busy.has(task.id)) return false
  busy.add(task.id)
  actionError.value = ''
  try {
    await operation(`/schedules/${task.id}`)
    await load()
    return true
  } catch (error) {
    actionError.value = error.response?.data?.error || '操作失败'
    return false
  } finally {
    busy.delete(task.id)
  }
}
function runNow(task) { return mutate(task, url => api.post(`${url}/run`)) }
function toggle(task) { return mutate(task, url => api.post(url, { enabled: !task.enabled })) }
function remove(task) { return mutate(task, url => api.delete(url)) }
async function saveTime(task) {
  if (!editing.value || !validTime(editing.value.time)) return
  if (await mutate(task, url => api.post(`${url}/time`, { daily_time: editing.value.time }))) editing.value = null
}
onMounted(() => {
  void load()
  timer = setInterval(() => {
    if (!document.hidden && !loading.value && !creating.value && !busy.size) void load()
  }, 10000)
})
onUnmounted(() => { disposed = true; clearInterval(timer); request?.abort() })
</script>

<style scoped>
.yt-page-title { margin: 0; }
.schedules-grid { display: grid; grid-template-columns: minmax(260px, 340px) minmax(0, 1fr); gap: 16px; align-items: start; }
.schedules-grid > * { min-width: 0; }
.create-actions { display: flex; justify-content: flex-end; margin-top: 12px; }
.field-note { color: var(--yt-ink-3); font-size: 13px; margin: 0 0 12px; }
.time-input { max-width: 100%; padding: 6px 8px; border: 1px solid var(--yt-card-border); border-radius: 6px; color: var(--yt-ink-1); background: var(--yt-surface); font: inherit; }
.schedule-row { padding: 14px 0; border-bottom: 1px solid var(--yt-card-border); }
.schedule-row:first-of-type { padding-top: 0; }
.schedule-row:last-of-type { border-bottom: 0; }
.schedule-row-head, .schedule-times, .schedule-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 14px; }
.schedule-row-head { justify-content: space-between; }
.schedule-name { min-width: 0; display: grid; gap: 3px; overflow-wrap: anywhere; }
.schedule-name span, .schedule-times dt { color: var(--yt-ink-3); font-size: 12px; }
.schedule-times { margin: 8px 0; }
.schedule-times div { display: flex; align-items: baseline; gap: 6px; }
.schedule-times dd { margin: 0; font-size: 13px; }
.schedule-result { margin: 8px 0; color: var(--yt-ink-2); font-size: 13px; }
.schedule-result summary { cursor: pointer; }
.schedule-result p { white-space: pre-wrap; overflow-wrap: anywhere; }
.schedule-list :deep(.n-pagination) { margin-top: 12px; }
@media (max-width: 810px) { .schedules-grid { grid-template-columns: minmax(0, 1fr); } }
</style>
