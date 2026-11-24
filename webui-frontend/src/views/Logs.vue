<template>
  <div class="yt-page runtime-page">
    <header class="yt-page-head">
      <div class="yt-page-head-left"><span class="yt-page-title">运行控制台</span><span class="yt-page-sub">日志与运行状态</span></div>
      <n-button secondary size="small" :loading="loading" @click="refresh">刷新</n-button>
    </header>
    <n-alert v-if="error" type="error" :bordered="false" closable @close="error = ''">{{ error }}</n-alert>
    <n-alert v-if="statusError" type="warning" :bordered="false">{{ statusError }}</n-alert>
    <section class="runtime-metrics" aria-label="主进程状态">
      <article><span class="metric-icon"><yt-icon name="terminal" /></span><div><span>主进程</span><strong class="mono">{{ status?.process?.pid ?? '—' }}<small>PID</small></strong></div></article>
      <article><span class="metric-icon"><yt-icon name="trace" /></span><div><span>运行时长</span><strong class="mono">{{ duration(status?.uptime_secs) }}</strong></div></article>
      <article><span class="metric-icon"><yt-icon name="models" /></span><div><span>CPU 占用</span><strong class="mono">{{ percent(status?.process?.cpu_percent) }}</strong></div></article>
      <article><span class="metric-icon"><yt-icon name="mcp" /></span><div><span>内存占用</span><strong class="mono">{{ bytes(status?.process?.memory_bytes) }}</strong></div></article>
    </section>

    <div class="runtime-tabs">
      <n-tabs v-model:value="tab" type="line" size="small" role="tablist" aria-label="控制台视图">
        <n-tab name="logs" role="tab" tabindex="0" :aria-selected="tab === 'logs'" @keydown.enter="tab = 'logs'" @keydown.space.prevent="tab = 'logs'">运行日志</n-tab>
        <n-tab name="process" role="tab" tabindex="0" :aria-selected="tab === 'process'" @keydown.enter="tab = 'process'" @keydown.space.prevent="tab = 'process'">进程与组件</n-tab>
      </n-tabs>
      <span class="capture-state"><i :class="{ live: tab === 'process' || (!paused && !date) }" />{{ tab === 'process' ? '每 10 秒更新' : date ? '历史记录' : paused ? '已暂停更新' : '每 2 秒更新' }}</span>
    </div>

    <template v-if="tab === 'logs'">
      <form class="yt-toolbar log-filters" @submit.prevent="reset">
        <n-select v-model:value="level" size="small" aria-label="日志级别" :options="levelOptions" placeholder="全部级别" clearable @update:value="reset" />
        <n-select v-model:value="target" size="small" aria-label="日志模块" :options="targetOptions" placeholder="全部模块" clearable filterable @update:value="reset" />
        <n-input v-model:value="search" size="small" :input-props="{ 'aria-label': '搜索日志' }" placeholder="搜索日志内容或上下文" clearable />
        <label class="log-date">日期<input v-model="date" aria-label="日志日期" type="date" @change="reset" /></label>
        <n-button attr-type="submit" size="small" secondary type="primary">筛选</n-button>
        <n-button v-if="date || level || target || search" size="small" text @click="clearFilters">重置</n-button>
      </form>
      <div v-if="storage?.dropped_writes || storage?.disk_errors || gap" class="log-storage-warning" role="status">
        {{ gap ? '更新期间部分记录已移出缓存，请检查磁盘日志。' : '' }}
        <template v-if="storage?.dropped_writes">{{ storage.dropped_writes }} 条日志未能进入写盘队列。</template>
        <template v-if="storage?.disk_errors">日志落盘出现错误，请检查磁盘空间和权限。</template>
      </div>
      <div class="log-workspace">
        <section class="log-list" aria-label="运行日志列表">
          <header class="log-list-head"><h2>日志记录<span class="record-count mono">{{ logs.length }}</span></h2><n-button size="tiny" secondary :aria-label="paused ? '恢复更新' : '暂停更新'" @click="paused = !paused">{{ paused ? '恢复更新' : '暂停更新' }}</n-button></header>
          <div class="log-columns" aria-hidden="true"><span>时间</span><span>级别</span><span>来源模块</span><span>日志内容</span></div>
          <div class="log-list-body" :aria-busy="loading">
            <button v-for="entry in logs" :key="entry.id" class="log-row" :class="[entry.level.toLowerCase(), { selected: selected?.id === entry.id }]" :aria-pressed="selected?.id === entry.id" @click="selected = entry">
              <time :datetime="new Date(entry.ts).toISOString()"><span>{{ day(entry.ts) }}</span><span>{{ time(entry.ts) }}</span></time>
              <span class="log-level" :class="entry.level.toLowerCase()">{{ entry.level }}</span>
              <span class="log-target" :title="entry.target">{{ moduleName(entry.target) }}</span>
              <span class="log-content">{{ entry.message }}</span>
            </button>
            <div v-if="!logs.length" class="log-empty"><empty-state :title="loading ? '正在读取日志' : '暂无匹配日志'" /><p>{{ loading ? '连接运行记录中…' : '调整筛选条件或等待新的运行记录' }}</p></div>
          </div>
          <footer class="log-list-footer"><span>采集 {{ storage?.minimum_level || '—' }} · {{ storage?.persisting ? '已开启落盘' : '内存缓存' }}</span><n-button v-if="hasMore" size="tiny" text :loading="loading" @click="loadOlder">加载更早记录</n-button><span v-else>最近 {{ storage?.capacity ?? '—' }} 条缓存</span></footer>
        </section>

        <aside class="log-detail" aria-label="日志详情">
          <template v-if="selected">
            <header><span class="detail-kicker">{{ selected.level === 'ERROR' ? '错误详情' : '日志详情' }}</span><n-button text size="small" @click="copy">{{ copied ? '已复制' : '复制记录' }}</n-button></header>
            <div class="detail-summary"><span class="log-level" :class="selected.level.toLowerCase()">{{ selected.level }}</span><time>{{ dateTime(selected.ts) }}</time></div>
            <h2>{{ selected.message }}</h2>
            <dl><div><dt>来源模块</dt><dd class="mono">{{ selected.target }}</dd></div><div v-if="selected.file"><dt>发生位置</dt><dd class="mono">{{ selected.file }}<template v-if="selected.line">:{{ selected.line }}</template></dd></div></dl>
            <template v-if="Object.keys(selected.fields || {}).length"><h3>上下文与异常</h3><div v-for="(value, name) in selected.fields" :key="name" class="log-field"><span>{{ name }}</span><pre>{{ value }}</pre></div></template>
          </template>
          <div v-else class="detail-empty"><empty-state title="选择一条日志" /><p>查看来源、异常信息和关联上下文</p></div>
        </aside>
      </div>
    </template>

    <template v-else>
      <div class="process-heading"><h2>后台组件</h2><span>累计重启 {{ status?.total_restarts ?? '—' }} 次</span></div>
      <section class="component-grid">
        <article v-for="component in status?.components || []" :key="component.name" class="component-card">
          <div class="component-head"><span class="component-icon"><yt-icon name="mcp" :size="19" /></span><span class="component-state" :class="component.state">{{ stateLabel(component.state) }}</span></div>
          <h3>{{ componentLabel(component.name) }}</h3><p class="mono">{{ component.name }}</p>
          <footer><span>重启 <b>{{ component.restarts }}</b> 次</span><span>{{ dateTime(component.changed_at * 1000) }}</span></footer>
          <p v-if="component.reason" class="component-reason">{{ reasonLabel(component.reason) }}</p>
        </article>
      </section>
      <empty-state v-if="status && !status.components?.length" title="暂无受监督组件记录" />
      <div class="process-heading"><h2>平台与执行环境</h2></div>
      <div class="environment-list"><div v-for="platform in status?.platforms || []" :key="platform.id"><strong>{{ platform.id === 'onebot' ? 'OneBot / NapCat' : platform.id === 'qq_official' ? 'QQ 官方机器人' : platform.id }}</strong><span :class="{ 'text-green': platform.state === 'online' }">{{ ({ online: '已连接', offline: '等待连接', disabled: '未启用', error: '配置异常' })[platform.state] || platform.state }}</span></div><div><strong>Bash 沙箱</strong><span>{{ status?.bash_registered ? '已注册' : '未启用' }}</span></div></div>
      <div class="process-heading"><h2>主进程的子进程</h2><span>当前运行记录</span></div>
      <div class="child-processes"><div class="child-header"><span>进程</span><span>PID</span><span>内存</span><span>运行时间</span></div><div v-for="child in status?.process?.children || []" :key="child.pid" class="child-row"><strong>{{ child.name }}</strong><span class="mono">{{ child.pid }}</span><span class="mono">{{ bytes(child.memory_bytes) }}</span><span class="mono">{{ duration(child.run_time_secs) }}</span></div><p v-if="status && !status.process?.children?.length" class="child-empty">当前没有可查询的子进程</p></div>
    </template>
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { api } from '../api'
import YtIcon from '../components/YtIcon.vue'
import EmptyState from '../components/EmptyState.vue'

/** @type {import('vue').Ref<any[]>} */
const logs = ref([])
/** @type {import('vue').Ref<any>} */
const selected = ref(null), status = ref(null), storage = ref(null)
const tab = ref('logs'), level = ref(null), target = ref(null), search = ref(''), date = ref('')
const paused = ref(false), loading = ref(false), hasMore = ref(false), gap = ref(false), copied = ref(false)
const error = ref(''), statusError = ref(''), targets = ref([])
const levelOptions = ['ERROR', 'WARN', 'INFO', 'DEBUG', 'TRACE'].map(value => ({ label: value, value }))
const targetOptions = computed(() => targets.value.map(value => ({ label: value, value })))
let cursor = 0, generation = 0, timer, controller, stopped = false, statusPending = false
let activeParams = new URLSearchParams()
function parameters() {
  const params = new URLSearchParams({ limit: '100' })
  if (level.value) params.set('level', level.value)
  if (target.value) params.set('target', target.value)
  if (search.value) params.set('search', search.value)
  if (date.value) {
    const start = new Date(`${date.value}T00:00:00`), end = new Date(start)
    end.setDate(end.getDate() + 1)
    params.set('from_ts', String(start.getTime())); params.set('until_ts', String(end.getTime()))
  }
  return params
}
async function fetchLogs(mode = 'reset') {
  if (loading.value) return
  const current = generation
  const params = new URLSearchParams(activeParams)
  if (mode === 'live' && cursor) params.set('after_id', String(cursor))
  if (mode === 'older' && logs.value.length) params.set('before_id', String(Math.min(...logs.value.map(entry => entry.id))))
  loading.value = true
  controller = new AbortController()
  try {
    const { data } = await api.get(`/logs?${params}`, { signal: controller.signal })
    if (stopped || current !== generation) return
    const entries = data.entries || []
    if (mode === 'reset' || (mode === 'live' && !cursor)) {
      logs.value = entries; hasMore.value = data.has_more
      cursor = data.latest_id || 0; selected.value = entries[0] || null
    } else {
      logs.value = [...new Map([...logs.value, ...entries].map(entry => [entry.id, entry])).values()].sort((a, b) => b.id - a.id).slice(0, 2000)
      if (mode === 'older') hasMore.value = data.has_more
      else cursor = entries.length ? Math.max(...entries.map(entry => entry.id)) : data.latest_id || cursor
    }
    gap.value = gap.value || Boolean(data.gap)
    storage.value = data.storage; targets.value = data.targets || []; error.value = ''
  } catch (e) {
    if (!stopped && current === generation && e.code !== 'ERR_CANCELED') error.value = e.response?.data?.error || '读取运行日志失败，请稍后刷新'
  } finally { if (current === generation) loading.value = false }
}
function reset() {
  activeParams = parameters()
  generation++; controller?.abort(); loading.value = false; hasMore.value = false; cursor = 0
  logs.value = []; selected.value = null; gap.value = false
  return fetchLogs()
}
function clearFilters() { level.value = null; target.value = null; search.value = ''; date.value = ''; reset() }
function loadOlder() { return fetchLogs('older') }
async function fetchStatus() {
  if (statusPending) return
  statusPending = true
  try { const { data } = await api.get('/runtime/status'); if (!stopped) { status.value = data; statusError.value = '' } }
  catch { if (!stopped) statusError.value = '运行状态暂时不可用' }
  finally { statusPending = false }
}
function refresh() { reset(); fetchStatus() }
async function copy() {
  try { await navigator.clipboard.writeText(JSON.stringify(selected.value, null, 2)); copied.value = true; setTimeout(() => { copied.value = false }, 1500) }
  catch { error.value = '复制失败，可选中详情中的文字手动复制' }
}
function duration(value) { if (value == null) return '—'; const hours = Math.floor(value / 3600), minutes = Math.floor(value % 3600 / 60); return hours ? `${hours}h ${minutes}m` : `${minutes}m ${Math.floor(value % 60)}s` }
function bytes(value) { return value == null ? '—' : `${(value / 1024 / 1024).toFixed(1)} MB` }
function percent(value) { return value == null ? '—' : `${value.toFixed(1)}%` }
function time(value) { return new Date(value).toLocaleTimeString('zh-CN', { hour12: false }) }
function day(value) { return new Date(value).toLocaleDateString('zh-CN', { month: '2-digit', day: '2-digit' }) }
function dateTime(value) { return new Date(value).toLocaleString('zh-CN', { hour12: false }) }
function moduleName(value) { return value.split('::').slice(-2).join(' / ') }
function stateLabel(value) { return ({ starting: '启动中', running: '运行中', backoff: '等待重启', stopped: '已停止', failed: '启动失败' })[value] || value }
function componentLabel(value) { return ({ tracer: '事件记录', pipeline: '对话管线', task_runner: '任务执行器', scheduled_tools: '定时调度', steal_listener: '表情采集', meme_review: '图库审核' })[value] || value }
function reasonLabel(value) { return ({ cancelled: '组件已取消', restart_limit: '重启次数达到上限，请检查错误日志', component_exited: '组件退出，正在退避重启' })[value] || '状态需要核实' }
onMounted(() => { refresh(); let ticks = 0; timer = setInterval(() => { if (!paused.value && !date.value && tab.value === 'logs') fetchLogs('live'); if (++ticks % 5 === 0) fetchStatus() }, 2000) })
onUnmounted(() => { stopped = true; clearInterval(timer); controller?.abort() })
</script>

<style scoped>
.runtime-metrics {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 12px;
}
.runtime-metrics article {
  display: flex;
  gap: 14px;
  align-items: center;
  padding: 18px 20px;
  background: var(--yt-surface);
  border: 1px solid var(--yt-card-border);
  border-radius: 12px;
  box-shadow: var(--yt-card-shadow);
  min-width: 0;
}
.metric-icon {
  width: 36px;
  height: 36px;
  flex: none;
  display: grid;
  place-items: center;
  border-radius: 9px;
  color: var(--yt-ink-3);
  background: var(--yt-soft-bg);
}
.runtime-metrics article div > span {
  color: var(--yt-ink-3);
  font-size: 12px;
}
.runtime-metrics strong {
  display: block;
  font-size: 23px;
  line-height: 1.6;
  font-weight: 600;
  color: var(--yt-ink-1);
}
.runtime-metrics small {
  font-size: 11px;
  font-weight: 400;
  color: var(--yt-ink-3);
  margin-left: 8px;
}
.runtime-tabs {
  position: relative;
}
.capture-state {
  position: absolute;
  right: 0;
  top: 8px;
  font-size: 12px;
  display: flex;
  gap: 7px;
  align-items: center;
  color: var(--yt-ink-3);
}
.capture-state i {
  width: 6px;
  height: 6px;
  background: var(--yt-ink-3);
  border-radius: 50%;
}
.capture-state i.live {
  background: var(--yt-ok);
}
.log-filters > .n-select {
  width: 130px;
}
.log-filters > .n-select:nth-child(2) {
  width: 200px;
}
.log-filters > .n-input {
  flex: 1;
  min-width: 180px;
}
.log-date {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  color: var(--yt-ink-3);
  font-size: 12px;
}
.log-date input {
  background: var(--yt-surface);
  color: var(--yt-ink-2);
  border: 1px solid var(--yt-card-border);
  border-radius: 6px;
  font: inherit;
  height: 28px;
  padding: 0 8px;
  min-width: 0;
  color-scheme: light;
}
:global(html.dark) .log-date input {
  color-scheme: dark;
}
.log-workspace {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 340px;
  gap: 16px;
  align-items: start;
}
.log-list, .log-detail {
  min-width: 0;
  color: var(--yt-ink-1);
  background: var(--yt-surface);
  border: 1px solid var(--yt-card-border);
  border-radius: 12px;
  box-shadow: var(--yt-card-shadow);
  overflow: hidden;
}
.log-list-head, .log-detail header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 16px 18px;
  border-bottom: 1px solid var(--yt-card-border);
}
.log-list-head h2 {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 600;
  margin: 0;
}
.record-count {
  color: var(--yt-ink-3);
  background: var(--yt-soft-bg);
  border-radius: 5px;
  padding: 1px 6px;
  font-size: 11px;
  font-weight: 400;
}
.log-columns, .log-row {
  display: grid;
  grid-template-columns: 72px 58px minmax(90px, .7fr) minmax(0, 2fr);
  gap: 14px;
  padding: 12px 18px;
}
.log-columns {
  color: var(--yt-ink-3);
  background: var(--yt-elevated);
  border-bottom: 1px solid var(--yt-card-border);
  font-size: 12px;
}
.log-list-body {
  height: min(60vh, 640px);
  min-height: 320px;
  overflow: auto;
  scrollbar-color: var(--yt-card-border-hover) transparent;
  scrollbar-width: thin;
}
.log-row {
  width: 100%;
  text-align: left;
  align-items: start;
  background: transparent;
  border: 0;
  box-shadow: inset 0 -1px var(--yt-card-border);
  color: var(--yt-ink-2);
  font: inherit;
  font-size: 13px;
  line-height: 1.7;
  cursor: pointer;
  position: relative;
}
.log-row:hover {
  background: var(--yt-hover-bg);
}
.log-row.selected {
  background: var(--yt-active-bg);
}
.log-row.selected::before {
  content: '';
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
  background: var(--yt-primary);
}
.log-row time {
  display: flex;
  flex-direction: column;
  color: var(--yt-ink-3);
  font: 11px/1.8 var(--yt-mono);
}
.log-row time span:last-child {
  color: var(--yt-ink-2);
  font-size: 12px;
}
.log-level {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  justify-self: start;
  color: var(--yt-ink-2);
  background: var(--yt-soft-bg);
  border-radius: 5px;
  padding: 2px 7px;
  font: 500 10px/1.8 var(--yt-mono);
}
.log-level.info {
  color: var(--yt-primary);
  background: var(--yt-active-bg);
}
.log-level.error {
  color: var(--yt-danger);
  background: var(--yt-danger-soft);
}
.log-level.warn {
  color: var(--yt-jk-num);
  background: var(--yt-warning-soft);
}
.log-level.debug {
  color: var(--yt-violet-ink);
  background: var(--yt-violet-soft);
}
.log-target {
  min-width: 0;
  overflow-wrap: anywhere;
  color: var(--yt-ink-3);
  font: 11px/1.8 var(--yt-mono);
}
.log-content {
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  overflow: hidden;
  overflow-wrap: anywhere;
  white-space: pre-wrap;
}
.log-list-footer {
  padding: 12px 18px;
  border-top: 1px solid var(--yt-card-border);
  font-size: 11px;
  color: var(--yt-ink-3);
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}
.log-empty {
  padding: 70px 16px;
}
.log-empty p, .detail-empty p {
  text-align: center;
  color: var(--yt-ink-3);
  font-size: 12px;
  line-height: 1.8;
  margin: 0;
}
.log-detail {
  position: sticky;
  top: 20px;
  max-height: 80vh;
  overflow: auto;
  padding: 0 20px 20px;
}
.log-detail header {
  margin: 0 -20px;
  padding: 14px 20px;
}
.detail-kicker {
  font-size: 14px;
  font-weight: 600;
}
.detail-summary {
  display: flex;
  gap: 10px;
  align-items: center;
  flex-wrap: wrap;
  margin-top: 20px;
}
.detail-summary time {
  font: 11px var(--yt-mono);
  color: var(--yt-ink-3);
}
.log-detail h2 {
  font-size: 15px;
  font-weight: 600;
  line-height: 1.8;
  overflow-wrap: anywhere;
  margin: 12px 0 20px;
  white-space: pre-wrap;
}
.log-detail dl {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding-top: 18px;
  border-top: 1px solid var(--yt-card-border);
}
.log-detail dt {
  font-size: 12px;
  color: var(--yt-ink-3);
  margin-bottom: 6px;
}
.log-detail dd {
  margin: 0;
  font-size: 12px;
  overflow-wrap: anywhere;
  color: var(--yt-ink-2);
}
.log-detail h3 {
  font-size: 13px;
  margin: 24px 0 12px;
  font-weight: 600;
}
.log-field {
  margin: 12px 0 0;
  border: 1px solid var(--yt-card-border);
  border-radius: 8px;
  overflow: hidden;
}
.log-field > span {
  display: block;
  padding: 8px 12px;
  font: 11px var(--yt-mono);
  color: var(--yt-ink-3);
  background: var(--yt-elevated);
  border-bottom: 1px solid var(--yt-card-border);
}
.log-field pre {
  margin: 0;
  padding: 12px;
  font: 12px/1.8 var(--yt-mono);
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  color: var(--yt-ink-2);
}
.detail-empty {
  padding: 80px 0;
}
.log-storage-warning {
  background: var(--yt-warning-soft);
  color: var(--yt-ink-2);
  padding: 10px 14px;
  border-radius: 8px;
  font-size: 12px;
}
.process-heading {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  margin: 6px 0 -2px;
}
.process-heading h2 {
  font-size: 14px;
  margin: 0;
}
.process-heading > span {
  font-size: 12px;
  color: var(--yt-ink-3);
}
.component-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 16px;
}
.component-card {
  min-width: 0;
  padding: 20px;
  border: 1px solid var(--yt-card-border);
  border-radius: 12px;
  background: var(--yt-surface);
  box-shadow: var(--yt-card-shadow);
}
.component-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.component-icon {
  color: var(--yt-ink-3);
}
.component-state {
  font-size: 11px;
  color: var(--yt-ink-3);
  background: var(--yt-soft-bg);
  padding: 3px 8px;
  border-radius: 5px;
}
.component-state.running {
  color: var(--yt-ok);
  background: var(--yt-ok-soft);
}
.component-state.backoff, .component-state.starting {
  color: var(--yt-jk-num);
  background: var(--yt-warning-soft);
}
.component-state.failed {
  color: var(--yt-danger);
  background: var(--yt-danger-soft);
}
.component-card h3 {
  font-size: 14px;
  margin: 16px 0 4px;
}
.component-card > p {
  font-size: 12px;
  color: var(--yt-ink-3);
  margin: 0;
}
.component-card footer {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  gap: 8px;
  font-size: 11px;
  color: var(--yt-ink-3);
  margin-top: 20px;
  padding-top: 12px;
  border-top: 1px solid var(--yt-card-border);
}
.component-card .component-reason {
  color: var(--yt-jk-num);
  font-size: 12px;
  margin-top: 10px;
}
.environment-list, .child-processes {
  border: 1px solid var(--yt-card-border);
  background: var(--yt-surface);
  border-radius: 12px;
  overflow: hidden;
  box-shadow: var(--yt-card-shadow);
}
.environment-list > div {
  display: flex;
  justify-content: space-between;
  padding: 16px 20px;
  font-size: 13px;
  border-bottom: 1px solid var(--yt-card-border);
}
.environment-list > div:last-child {
  border: 0;
}
.environment-list span {
  color: var(--yt-ink-3);
}
.environment-list .text-green {
  color: var(--yt-ok);
}
.child-header, .child-row {
  display: grid;
  grid-template-columns: 2fr 1fr 1fr 1fr;
  padding: 14px 20px;
  gap: 10px;
  font-size: 12px;
}
.child-header {
  color: var(--yt-ink-3);
  background: var(--yt-elevated);
}
.child-row {
  border-top: 1px solid var(--yt-card-border);
}
.child-row strong {
  font-weight: 500;
  overflow-wrap: anywhere;
}
.child-empty {
  padding: 22px;
  text-align: center;
  color: var(--yt-ink-3);
  font-size: 12px;
}
button:focus-visible, input:focus-visible {
  outline: 2px solid var(--yt-primary);
  outline-offset: -2px;
}
@media (min-width: 1600px) {
  .log-workspace {
    grid-template-columns: minmax(0, 1fr) 400px;
  }
}
@media (max-width: 1150px) {
  .runtime-metrics article {
    padding: 14px;
    gap: 10px;
  }
  .runtime-metrics strong {
    font-size: 20px;
  }
  .log-workspace {
    grid-template-columns: minmax(0, 1fr);
  }
  .log-detail {
    position: static;
    max-height: none;
  }
  .component-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
@media (max-width: 650px) {
  .runtime-metrics {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
  }
  .metric-icon {
    width: 30px;
    height: 30px;
  }
  .runtime-tabs {
    padding-bottom: 26px;
  }
  .capture-state {
    top: auto;
    bottom: 0;
  }
  .log-filters > .n-select, .log-filters > .n-select:nth-child(2) {
    width: calc(50% - 5px);
  }
  .log-filters > .n-input {
    flex-basis: 100%;
  }
  .log-columns {
    display: none;
  }
  .log-row {
    grid-template-columns: 64px minmax(0, 1fr) auto;
    gap: 6px 12px;
    padding: 14px;
  }
  .log-row time {
    grid-row: 1 / 3;
  }
  .log-target {
    grid-row: 1;
    grid-column: 2;
    align-self: center;
  }
  .log-row .log-level {
    grid-row: 1;
    grid-column: 3;
  }
  .log-content {
    grid-row: 2;
    grid-column: 2 / 4;
    -webkit-line-clamp: 3;
  }
  .log-list-head, .log-list-footer {
    padding: 12px 14px;
  }
  .component-grid {
    grid-template-columns: minmax(0, 1fr);
  }
  .child-header, .child-row {
    grid-template-columns: minmax(0, 1.2fr) repeat(3, minmax(0, 1fr));
    padding: 12px;
    font-size: 11px;
  }
}
</style>
