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
      <article><span class="metric-icon violet"><yt-icon name="trace" /></span><div><span>运行时长</span><strong class="mono">{{ duration(status?.uptime_secs) }}</strong></div></article>
      <article><span class="metric-icon cyan"><yt-icon name="models" /></span><div><span>CPU 占用</span><strong class="mono">{{ percent(status?.process?.cpu_percent) }}</strong></div></article>
      <article><span class="metric-icon green"><yt-icon name="mcp" /></span><div><span>内存占用</span><strong class="mono">{{ bytes(status?.process?.memory_bytes) }}</strong></div></article>
    </section>

    <div class="runtime-tabs" role="tablist" aria-label="控制台视图">
      <button role="tab" :aria-selected="tab === 'logs'" :class="{ active: tab === 'logs' }" @click="tab = 'logs'"><yt-icon name="terminal" :size="17" />运行日志</button>
      <button role="tab" :aria-selected="tab === 'process'" :class="{ active: tab === 'process' }" @click="tab = 'process'"><yt-icon name="mcp" :size="17" />进程与组件</button>
      <span class="spacer" /><span class="capture-state"><i :class="{ live: tab === 'process' || (!paused && !date) }" />{{ tab === 'process' ? '每 10 秒更新' : date ? '历史记录' : paused ? '已暂停更新' : '每 2 秒更新' }}</span>
    </div>

    <template v-if="tab === 'logs'">
      <form class="log-filters" @submit.prevent="reset">
        <n-select v-model:value="level" aria-label="日志级别" :options="levelOptions" placeholder="全部级别" clearable @update:value="reset" />
        <n-select v-model:value="target" aria-label="日志模块" :options="targetOptions" placeholder="全部模块" clearable filterable @update:value="reset" />
        <n-input v-model:value="search" :input-props="{ 'aria-label': '搜索日志' }" placeholder="搜索错误信息、关键词…" clearable />
        <label class="log-date"><input v-model="date" aria-label="日志日期" type="date" @change="reset" /></label>
        <n-button attr-type="submit" secondary type="primary">筛选</n-button>
        <n-button v-if="date || level || target || search" text @click="clearFilters">重置</n-button>
      </form>
      <div v-if="storage?.dropped_writes || storage?.disk_errors || gap" class="log-storage-warning" role="status">
        {{ gap ? '更新期间部分记录已移出缓存，请检查磁盘日志。' : '' }}
        <template v-if="storage?.dropped_writes">{{ storage.dropped_writes }} 条日志未能进入写盘队列。</template>
        <template v-if="storage?.disk_errors">日志落盘出现错误，请检查磁盘空间和权限。</template>
      </div>
      <div class="log-workspace">
        <section class="console" aria-label="运行日志列表">
          <div class="console-toolbar"><span class="console-title"><i /><i /><i /><span>runtime.log</span></span><span class="spacer" /><span>{{ logs.length }} 条</span><button :aria-label="paused ? '恢复更新' : '暂停更新'" @click="paused = !paused">{{ paused ? '继续' : '暂停' }}</button></div>
          <div class="console-columns"><span>时间</span><span>级别</span><span>模块 / 信息</span></div>
          <div class="console-body" :aria-busy="loading">
            <button v-for="entry in logs" :key="entry.id" class="log-row" :class="[entry.level.toLowerCase(), { selected: selected?.id === entry.id }]" @click="selected = entry">
              <time>{{ time(entry.ts) }}</time><span class="log-level">{{ entry.level }}</span><span class="log-content"><span class="log-target">{{ moduleName(entry.target) }}</span><span>{{ entry.message }}</span></span>
            </button>
            <div v-if="!logs.length" class="console-empty"><yt-icon name="terminal" :size="30" /><strong>{{ loading ? '正在读取日志' : '暂无匹配日志' }}</strong><span>{{ loading ? '连接运行记录中…' : '调整筛选条件或等待新的运行记录' }}</span></div>
          </div>
          <footer class="console-footer"><span><i />采集 {{ storage?.minimum_level || '—' }} · {{ storage?.persisting ? '已开启落盘' : '内存缓存' }}</span><button v-if="hasMore" :disabled="loading" @click="loadOlder">加载更早记录</button><span v-else>最近 {{ storage?.capacity ?? '—' }} 条缓存</span></footer>
        </section>

        <aside class="log-detail" aria-label="日志详情">
          <template v-if="selected">
            <header><span class="detail-kicker">{{ selected.level === 'ERROR' ? '错误详情' : '日志详情' }}</span><n-button text size="small" @click="copy">{{ copied ? '已复制' : '复制记录' }}</n-button></header>
            <div class="detail-summary"><span class="detail-level" :class="selected.level.toLowerCase()">{{ selected.level }}</span><time>{{ dateTime(selected.ts) }}</time></div>
            <h2>{{ selected.message }}</h2>
            <dl><div><dt>来源模块</dt><dd class="mono">{{ selected.target }}</dd></div><div v-if="selected.file"><dt>发生位置</dt><dd class="mono">{{ selected.file }}<template v-if="selected.line">:{{ selected.line }}</template></dd></div></dl>
            <template v-if="Object.keys(selected.fields || {}).length"><h3>上下文与异常</h3><div v-for="(value, name) in selected.fields" :key="name" class="log-field"><span>{{ name }}</span><pre>{{ value }}</pre></div></template>
          </template>
          <div v-else class="detail-empty"><yt-icon name="trace" :size="30" /><h2>查看发生了什么</h2><p>选择一条日志，查看来源、异常信息和关联上下文。</p></div>
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
    if (mode === 'reset') {
      logs.value = entries; hasMore.value = data.has_more
      cursor = data.latest_id || 0; selected.value = entries[0] || null
    } else {
      logs.value = [...new Map([...logs.value, ...entries].map(entry => [entry.id, entry])).values()].sort((a, b) => b.id - a.id).slice(0, 2000)
      if (mode === 'older') hasMore.value = data.has_more
      else cursor = entries.length ? Math.max(...entries.map(entry => entry.id)) : data.latest_id || cursor
    }
    gap.value = Boolean(data.gap)
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
function dateTime(value) { return new Date(value).toLocaleString('zh-CN', { hour12: false }) }
function moduleName(value) { return value.split('::').slice(-2).join(' / ') }
function stateLabel(value) { return ({ starting: '启动中', running: '运行中', backoff: '等待重启', stopped: '已停止', failed: '启动失败' })[value] || value }
function componentLabel(value) { return ({ tracer: '事件记录', pipeline: '对话管线', task_runner: '任务执行器', scheduled_tools: '定时调度', steal_listener: '表情采集', meme_review: '图库审核' })[value] || value }
function reasonLabel(value) { return ({ cancelled: '组件已取消', restart_limit: '重启次数达到上限，请检查错误日志', component_exited: '组件退出，正在退避重启' })[value] || '状态需要核实' }
onMounted(() => { refresh(); let ticks = 0; timer = setInterval(() => { if (!paused.value && !date.value && tab.value === 'logs') fetchLogs('live'); if (++ticks % 5 === 0) fetchStatus() }, 2000) })
onUnmounted(() => { stopped = true; clearInterval(timer); controller?.abort() })
</script>

<style scoped>
.runtime-page {
  max-width:1700px;
  margin:0 auto;
}
.runtime-metrics {
  display:grid;
  grid-template-columns:repeat(4,minmax(0,1fr));
  gap:16px;
  margin:22px 0 28px;
}
.runtime-metrics article {
  display:flex;
  gap:16px;
  align-items:center;
  padding:22px;
  background:var(--yt-surface);
  border:1px solid var(--yt-card-border);
  border-radius:14px;
  box-shadow:var(--yt-card-shadow);
}
.metric-icon {
  width:42px;
  height:42px;
  display:grid;
  place-items:center;
  border-radius:12px;
  color:var(--yt-primary);
  background:var(--yt-active-bg);
}
.metric-icon.violet {
  color:var(--yt-violet);
  background:var(--yt-violet-soft);
}
.metric-icon.cyan {
  color:var(--yt-cyan);
  background:var(--yt-cyan-soft);
}
.metric-icon.green {
  color:var(--yt-ok);
  background:var(--yt-ok-soft);
}
.runtime-metrics article div>span {
  color:var(--yt-ink-3);
  font-size:12px;
}
.runtime-metrics strong {
  display:block;
  font-size:25px;
  line-height:1.5;
  letter-spacing:-1px;
}
.runtime-metrics small {
  font-size:11px;
  color:var(--yt-ink-3);
  margin-left:9px;
  letter-spacing:0;
}
.runtime-tabs {
  display:flex;
  align-items:center;
  border-bottom:1px solid var(--yt-card-border);
  margin-bottom:20px;
  gap:25px;
}
.runtime-tabs>button {
  display:flex;
  gap:8px;
  align-items:center;
  background:none;
  border:0;
  padding:0 0 14px;
  color:var(--yt-ink-3);
  font-size:14px;
  border-bottom:2px solid transparent;
  cursor:pointer;
}
.runtime-tabs>button.active {
  color:var(--yt-primary);
  border-color:var(--yt-primary);
}
.capture-state {
  font-size:12px;
  display:flex;
  gap:8px;
  align-items:center;
  padding-bottom:14px;
  color:var(--yt-ink-3);
}
.capture-state i,.console-footer i {
  width:6px;
  height:6px;
  background:#64748b;
  border-radius:50%;
  display:inline-block;
}
.capture-state i.live,.console-footer i {
  background:#34d399;
  box-shadow:0 0 0 3px #34d39912;
}
.spacer {
  flex:1;
}
.log-filters {
  display:flex;
  gap:10px;
  margin-bottom:16px;
  align-items:center;
  flex-wrap:wrap;
}
.log-filters>.n-select {
  width:145px;
}
.log-filters>.n-select:nth-child(2) {
  width:215px;
}
.log-filters>.n-input {
  flex:1;
  min-width:180px;
}
.log-date {
  display:flex;
  align-items:center;
  border:1px solid var(--yt-card-border);
  border-radius:6px;
  background:var(--yt-surface);
  height:34px;
  padding:0 10px;
}
.log-date input {
  background:none;
  color:var(--yt-ink-2);
  border:none;
  font:inherit;
  width:136px;
  color-scheme:light dark;
}
.log-workspace {
  display:grid;
  grid-template-columns:minmax(0,1.7fr) minmax(300px,1fr);
  gap:20px;
  align-items:start;
}
.console {
  background:#101827;
  color:#cbd5e1;
  border:1px solid #253247;
  border-radius:13px;
  overflow:hidden;
  box-shadow:0 8px 24px #0f172a0a;
}
.console-toolbar {
  display:flex;
  gap:14px;
  align-items:center;
  padding:14px 18px;
  background:#172235;
  border-bottom:1px solid #273447;
  font-size:11px;
  color:#8797af;
}
.console-title {
  display:flex;
  gap:6px;
  align-items:center;
}
.console-title i {
  width:7px;
  height:7px;
  border-radius:50%;
  background:#52617b;
}
.console-title i:first-child {
  background:#f87171;
}
.console-title i:nth-child(2) {
  background:#fbbf24;
}
.console-title i:nth-child(3) {
  background:#34d399;
}
.console-title span {
  margin-left:10px;
  font-family:var(--yt-mono);
  color:#bfcee2;
}
.console button {
  font:inherit;
  cursor:pointer;
}
.console-toolbar button,.console-footer button {
  border:1px solid #394b65;
  border-radius:5px;
  background:transparent;
  color:#cbd5e1;
  padding:3px 9px;
}
.console-columns,.log-row {
  display:grid;
  grid-template-columns:72px 52px minmax(0,1fr);
  gap:12px;
}
.console-columns {
  padding:11px 18px;
  color:#75859d;
  border-bottom:1px solid #233046;
  font-size:11px;
}
.console-body {
  height:min(62vh,680px);
  min-height:340px;
  overflow:auto;
  scrollbar-color:#394b65 #101827;
}
.log-row {
  width:100%;
  text-align:left;
  background:none;
  border:0;
  border-left:3px solid transparent;
  color:#b5c2d6;
  padding:12px 15px;
  font-family:var(--yt-mono);
  font-size:11px;
  line-height:1.7;
  border-bottom:1px solid #ffffff05;
}
.log-row:hover {
  background:#ffffff04;
}
.log-row.selected {
  background:#2563eb15;
  border-left-color:#60a5fa;
}
.log-row.error {
  background:#ef444409;
}
.log-row.error.selected {
  background:#ef444415;
  border-left-color:#f87171;
}
.log-row time {
  color:#657892;
}
.log-level {
  color:#7dd3fc;
  font-size:10px;
  font-weight:700;
}
.error .log-level {
  color:#fb7185;
}
.warn .log-level {
  color:#fbbf24;
}
.debug .log-level {
  color:#a78bfa;
}
.log-content {
  min-width:0;
  overflow-wrap:anywhere;
  white-space:pre-wrap;
}
.log-target {
  display:block;
  color:#697f9f;
  font-size:10px;
  margin-bottom:3px;
}
.console-footer {
  padding:12px 18px;
  border-top:1px solid #273447;
  font-size:10px;
  color:#8394ad;
  display:flex;
  align-items:center;
  justify-content:space-between;
  gap:10px;
}
.console-footer i {
  margin-right:7px;
}
.console-empty {
  display:flex;
  flex-direction:column;
  align-items:center;
  justify-content:center;
  gap:12px;
  min-height:330px;
  color:#697f9f;
}
.console-empty strong {
  font-size:14px;
  font-weight:500;
  color:#b5c2d6;
}
.console-empty span {
  font-size:12px;
}
.log-detail {
  padding:22px;
  background:var(--yt-surface);
  border:1px solid var(--yt-card-border);
  border-radius:13px;
  min-height:360px;
  position:sticky;
  top:20px;
  max-height:85vh;
  overflow:auto;
}
.log-detail header {
  display:flex;
  align-items:center;
  justify-content:space-between;
  gap:10px;
  border-bottom:1px solid var(--yt-card-border);
  padding-bottom:15px;
}
.detail-kicker {
  font-size:13px;
  font-weight:600;
}
.detail-summary {
  display:flex;
  gap:12px;
  align-items:center;
  flex-wrap:wrap;
  margin-top:20px;
}
.detail-level {
  font:600 10px var(--yt-mono);
  background:var(--yt-active-bg);
  color:var(--yt-primary);
  padding:4px 8px;
  border-radius:4px;
}
.detail-level.error {
  color:var(--yt-danger);
  background:var(--yt-danger-soft);
}
.detail-level.warn {
  color:var(--yt-warning);
  background:var(--yt-warning-soft);
}
.detail-summary time {
  font-size:11px;
  color:var(--yt-ink-3);
}
.log-detail h2 {
  font-size:16px;
  font-weight:600;
  line-height:1.7;
  overflow-wrap:anywhere;
  margin:15px 0 24px;
  white-space:pre-wrap;
}
.log-detail dl {
  display:flex;
  flex-direction:column;
  gap:16px;
}
.log-detail dt {
  font-size:11px;
  color:var(--yt-ink-3);
  margin-bottom:6px;
}
.log-detail dd {
  margin:0;
  font-size:11px;
  overflow-wrap:anywhere;
  color:var(--yt-ink-2);
}
.log-detail h3 {
  font-size:12px;
  margin:25px 0 12px;
}
.log-field {
  margin:12px 0;
}
.log-field>span {
  font:11px var(--yt-mono);
  color:var(--yt-ink-3);
}
.log-field pre {
  margin:6px 0 0;
  padding:12px;
  background:var(--yt-code-bg);
  border-radius:7px;
  font:11px/1.8 var(--yt-mono);
  white-space:pre-wrap;
  overflow-wrap:anywhere;
  color:var(--yt-ink-2);
}
.detail-empty {
  padding:60px 15px;
  text-align:center;
  color:var(--yt-ink-3);
}
.detail-empty h2 {
  font-size:14px;
  margin:15px 0 7px;
  color:var(--yt-ink-2);
}
.detail-empty p {
  font-size:12px;
  line-height:1.8;
}
.log-storage-warning {
  background:var(--yt-warning-soft);
  color:var(--yt-ink-2);
  padding:10px 14px;
  margin-bottom:12px;
  border-radius:8px;
  font-size:12px;
}
.process-heading {
  display:flex;
  justify-content:space-between;
  align-items:center;
  margin:25px 0 14px;
}
.process-heading h2 {
  font-size:14px;
  margin:0;
}
.process-heading>span {
  font-size:12px;
  color:var(--yt-ink-3);
}
.component-grid {
  display:grid;
  grid-template-columns:repeat(3,minmax(0,1fr));
  gap:16px;
}
.component-card {
  padding:20px;
  border:1px solid var(--yt-card-border);
  border-radius:12px;
  background:var(--yt-surface);
}
.component-head {
  display:flex;
  align-items:center;
  justify-content:space-between;
}
.component-icon {
  color:var(--yt-ink-3);
}
.component-state {
  font-size:11px;
  color:var(--yt-ink-3);
  background:var(--yt-soft-bg);
  padding:3px 8px;
  border-radius:5px;
}
.component-state.running {
  color:var(--yt-ok);
  background:var(--yt-ok-soft);
}
.component-state.backoff,.component-state.starting {
  color:var(--yt-warning);
  background:var(--yt-warning-soft);
}
.component-state.failed {
  color:var(--yt-danger);
  background:var(--yt-danger-soft);
}
.component-card h3 {
  font-size:14px;
  margin:16px 0 4px;
}
.component-card>p {
  font-size:11px;
  color:var(--yt-ink-3);
  margin:0;
}
.component-card footer {
  display:flex;
  justify-content:space-between;
  gap:8px;
  font-size:10px;
  color:var(--yt-ink-3);
  margin-top:22px;
  padding-top:13px;
  border-top:1px solid var(--yt-card-border);
}
.component-card .component-reason {
  color:var(--yt-warning);
  font-size:11px;
  margin-top:10px;
}
.environment-list,.child-processes {
  border:1px solid var(--yt-card-border);
  background:var(--yt-surface);
  border-radius:12px;
  overflow:hidden;
}
.environment-list>div {
  display:flex;
  justify-content:space-between;
  padding:16px 20px;
  font-size:12px;
  border-bottom:1px solid var(--yt-card-border);
}
.environment-list>div:last-child {
  border:0;
}
.environment-list span {
  color:var(--yt-ink-3);
}
.environment-list .text-green {
  color:var(--yt-ok);
}
.child-header,.child-row {
  display:grid;
  grid-template-columns:2fr 1fr 1fr 1fr;
  padding:14px 20px;
  gap:10px;
  font-size:12px;
}
.child-header {
  color:var(--yt-ink-3);
  background:var(--yt-elevated);
  font-size:11px;
}
.child-row {
  border-top:1px solid var(--yt-card-border);
}
.child-row strong {
  font-weight:500;
  overflow-wrap:anywhere;
}
.child-empty {
  padding:22px;
  text-align:center;
  color:var(--yt-ink-3);
  font-size:12px;
}
button:focus-visible,input:focus-visible {
  outline:2px solid var(--yt-primary);
  outline-offset:2px;
}
@media(min-width:1650px) {
  .log-workspace {
    grid-template-columns:minmax(0,1.9fr) minmax(340px,1fr);
  }
}
@media(max-width:1150px) {
  .runtime-metrics {
    gap:10px;
  }
  .runtime-metrics article {
    padding:16px;
    gap:10px;
  }
  .runtime-metrics strong {
    font-size:21px;
  }
  .log-workspace {
    grid-template-columns:minmax(0,1fr);
  }
  .log-detail {
    position:static;
    max-height:none;
  }
  .component-grid {
    grid-template-columns:repeat(2,minmax(0,1fr));
  }
}
@media(max-width:650px) {
  .runtime-metrics {
    grid-template-columns:repeat(2,minmax(0,1fr));
    margin-top:15px;
  }
  .runtime-tabs {
    gap:16px;
  }
  .capture-state {
    font-size:10px;
  }
  .log-filters>.n-select,.log-filters>.n-select:nth-child(2) {
    width:calc(50% - 5px);
  }
  .log-filters>.n-input {
    flex-basis:100%;
  }
  .console-columns,.log-row {
    grid-template-columns:58px 42px minmax(0,1fr);
    gap:7px;
  }
  .log-row {
    padding:10px 8px;
    font-size:10px;
  }
  .console-columns {
    padding:10px;
  }
  .console-toolbar,.console-footer {
    padding:12px;
  }
  .component-grid {
    grid-template-columns:minmax(0,1fr);
  }
  .runtime-metrics article {
    padding:13px;
  }
  .metric-icon {
    width:34px;
    height:34px;
  }
  .console-body {
    min-height:320px;
  }
  .child-header,.child-row {
    padding:12px;
    font-size:10px;
  }
}
</style>
