<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">任务回放</span>
        <span class="yt-page-sub mono">{{ list.length }} 个任务</span>
      </div>
      <n-button size="small" secondary @click="load">刷新</n-button>
    </div>

    <div class="yt-toolbar">
      <n-select v-model:value="state" :options="stateOptions" clearable placeholder="任务状态" size="small" style="width: 160px" @update:value="load" />
    </div>

    <n-grid cols="1 l:2" :x-gap="16" :y-gap="16" responsive="screen">
      <n-gi>
        <n-card title="任务列表" size="small">
          <n-skeleton v-if="!firstLoaded" text :repeat="6" />
          <div v-else-if="list.length" class="task-list">
            <div v-for="t in list" :key="t.task_id" class="task-row" :class="{ active: cur?.task_id === t.task_id }" @click="open(t)">
              <n-tag size="tiny" :type="stateTag(t.state)" :bordered="false">{{ t.state }}</n-tag>
              <span class="task-goal">{{ t.goal }}</span>
              <span class="task-budget mono">{{ t.used_calls }}/{{ t.budget_max_calls }}</span>
            </div>
          </div>
          <empty-state v-else title="暂无任务记录" />
        </n-card>
      </n-gi>

      <n-gi>
        <n-card :title="cur ? `${cur.task_id} · ${brief?.goal || cur.goal}` : '执行时间轴'" size="small">
          <template v-if="cur">
            <n-alert v-if="loadError" type="error">{{ loadError }}</n-alert>
            <div v-if="runtime" class="yt-toolbar">
              <n-tag>{{ phaseLabel(runtime.phase) }}</n-tag>
              <n-tag>{{ deliveryLabel(runtime.delivery_state) }}</n-tag>
              <n-button v-if="cur.state === 'running' && !runtime.parent_id && !['cancelled','finished','failed'].includes(runtime.phase)" size="small" :disabled="controlling" @click="control(['paused','waiting_input'].includes(runtime.phase) ? 'resume' : 'pause')">{{ ['paused','waiting_input'].includes(runtime.phase) ? '继续' : '暂停' }}</n-button>
              <n-button v-if="cur.state === 'running' && !runtime.parent_id && !['cancelled','finished','failed'].includes(runtime.phase)" size="small" :disabled="controlling" @click="control('cancel')">取消任务</n-button>
            </div>
            <p v-if="runtime?.stop_reason">{{ runtime.stop_reason }}</p>
            <p v-if="runtime?.final_text">{{ runtime.final_text }}</p>
            <section v-if="brief" class="task-brief">
              <div class="brief-head"><strong>任务说明</strong><n-tag size="small">v{{ brief.revision }}</n-tag></div>
              <p>{{ brief.goal }}</p>
              <template v-for="work in brief.working_context || []" :key="work.context_id">
                <div v-for="condition in work.conditions || []" :key="condition.key" class="brief-condition">
                  <strong>{{ condition.key }}</strong><span>{{ condition.quote }}</span><span class="mono source">#{{ condition.source_msg_id }}</span>
                </div>
              </template>
              <n-collapse>
                <n-collapse-item v-if="brief.request" title="原始要求" name="request"><p class="brief-text">{{ brief.request.text }}</p><span class="mono source">#{{ brief.request.msg_id }} · {{ fmtTs(brief.request.ts) }}</span></n-collapse-item>
                <n-collapse-item v-if="brief.changes?.length" :title="`补充要求（${brief.changes.length}）`" name="changes"><div v-for="(change,index) in brief.changes" :key="index"><p class="brief-text">{{ change.text }}</p><span class="source">{{ change.source ? `#${change.source.msg_id}` : '管理员调整' }}</span></div></n-collapse-item>
                <n-collapse-item v-if="briefVersions.length" title="版本记录" name="versions"><p v-for="version in briefVersions" :key="version.revision">v{{ version.revision }} · {{ changeLabel(version.event_kind) }} <span v-if="version.source_msg_id" class="mono source">#{{ version.source_msg_id }}</span></p></n-collapse-item>
              </n-collapse>
            </section>
            <n-collapse v-if="invocations.length"><n-collapse-item title="工具执行记录" name="calls"><p v-for="call in invocations" :key="call.id">{{ call.tool }} · {{ toolStatus(call.status || call.state) }} · {{ fmtTs(call.started_at) }}<span v-if="call.call_id" class="mono source"> · {{ call.protocol }} / {{ call.call_id }} / v{{ call.revision }} · {{ call.elapsed_ms }}ms</span><span v-if="call.error?.message" class="tool-error">{{ call.error.message }}</span></p></n-collapse-item></n-collapse>
            <n-timeline v-if="events.length">
              <n-timeline-item v-for="e in events" :key="e.id" :title="`#${e.seq} ${e.kind}`" :content="textOf(e)" :time="fmtTs(e.ts)" />
            </n-timeline>
            <empty-state v-else title="暂无执行事件" />
          </template>
          <empty-state v-else title="选择左侧任务查看执行流水" />
        </n-card>
      </n-gi>
    </n-grid>
  </div>
</template>

<script setup>
import { onMounted, ref } from 'vue'
import { api } from '../api'
import { useRoute } from 'vue-router'
const route = useRoute()
import { fmtTs } from '../fmt'
import EmptyState from '../components/EmptyState.vue'

const list = ref([])
const cur = ref(null)
const events = ref([])
const state = ref(null)
const runtime=ref(null)
const invocations=ref([])
const brief=ref(null)
const briefVersions=ref([])
const loadError=ref('')
const controlling=ref(false)
let requestVersion=0
let listVersion=0
const firstLoaded = ref(false)
const stateOptions = ['pending', 'running', 'finished', 'failed', 'archived'].map((s) => ({ label: s, value: s }))

function stateTag(s) {
  return ({ running: 'success', finished: 'info', failed: 'error', archived: 'default', pending: 'warning' })[s] || 'default'
}
function textOf(e) {
  if (e.payload == null) return ''
  return typeof e.payload === 'string' ? e.payload : JSON.stringify(e.payload)
}

async function load() {
  const version=++listVersion
  const params = new URLSearchParams({ limit: '100' })
  if (state.value) params.set('state', state.value)
  const { data } = await api.get(`/tasks?${params}`)
  if(version!==listVersion)return
  list.value = data.tasks || []
  firstLoaded.value = true
  const requested=cur.value?.task_id || route.query.task
  if (requested && !list.value.some(t=>t.task_id===requested)) {
    const exact=await api.get(`/tasks?task_id=${encodeURIComponent(String(requested))}`)
    if(version!==listVersion)return
    list.value.unshift(...(exact.data.tasks || []))
  }
  const selected=requested ? list.value.find(t=>t.task_id===requested) : list.value[0]
  if(selected) await open(selected)
  else {++requestVersion;cur.value=null;runtime.value=null;brief.value=null;briefVersions.value=[];events.value=[];invocations.value=[]}
}
function toolStatus(value) { return ({started:'执行结果待确认',succeeded:'执行成功',rejected:'未执行',failed:'执行失败',unknown:'结果待核实',cancelled:'已中止'})[value] || value }
function phaseLabel(value) { return ({queued:'排队中',executing:'执行中',waiting_input:'等待补充',waiting_children:'等待子任务',paused:'已暂停',cancelled:'已取消',finished:'已完成',failed:'已停止'})[value] || value }
function deliveryLabel(value) { return ({none:'尚无待发送结果',pending:'等待整理回复',preparing:'整理回复中',sent:'已发送',failed:'回复失败',unknown:'送达待核实'})[value] || value }
function changeLabel(value) { return ({create:'创建',amend:'补充要求',pause:'暂停',resume:'继续',cancel:'取消',delegate:'子任务',ancestor_control:'父任务变更'})[value] || value }
async function control(action) {
  if (!cur.value || controlling.value) return
  controlling.value=true;loadError.value=''
  try { await api.post(`/tasks/${cur.value.task_id}/control`,{action}); await load() }
  catch(e) {loadError.value=e.response?.data?.error || '操作失败'}
  finally {controlling.value=false}
}
async function open(t) {
  cur.value=t;const version=++requestVersion;runtime.value=null;invocations.value=[];events.value=[];brief.value=null;briefVersions.value=[];loadError.value=''
  try {
    const [eventResponse,runtimeResponse]=await Promise.all([api.get(`/tasks/${t.task_id}/events`),api.get(`/tasks/${t.task_id}/runtime`)])
    if(version!==requestVersion)return
    events.value=eventResponse.data.events || [];runtime.value=runtimeResponse.data.runtime;invocations.value=runtimeResponse.data.invocations || [];brief.value=runtimeResponse.data.brief || null;briefVersions.value=runtimeResponse.data.brief_versions || []
  } catch(e) {if(version===requestVersion)loadError.value=e.response?.data?.error || '加载失败'}
}
onMounted(load)
</script>

<style scoped>
.tool-error { display:block; color:var(--yt-ink-2); overflow-wrap:anywhere; }
.task-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: calc(100vh - 260px);
  overflow-y: auto;
}
.task-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border: 1px solid var(--yt-card-border);
  border-radius: 8px;
  cursor: pointer;
  transition: background 0.14s ease, border-color 0.14s ease;
}
.task-row:hover {
  background: var(--yt-elevated);
}
.task-row.active {
  background: var(--yt-active-bg);
  border-color: var(--yt-primary);
}
.task-goal {
  flex: 1;
  min-width: 0;
  font-size: 13.5px;
  color: var(--yt-ink-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.task-budget {
  font-size: 12px;
  color: var(--yt-ink-3);
}
.task-brief { margin: 16px 0; padding: 14px; border: 1px solid var(--yt-card-border); border-radius: 8px; }
.brief-head,.brief-condition { display: flex; align-items: baseline; gap: 10px; }
.brief-condition { margin: 8px 0; flex-wrap: wrap; }
.brief-head { align-items: center; }
.brief-text { white-space: pre-wrap; overflow-wrap: anywhere; }
.source { font-size: 12px; color: var(--yt-ink-3); }
</style>
