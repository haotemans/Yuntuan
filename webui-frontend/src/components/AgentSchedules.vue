<template>
  <section aria-label="云团定时任务" class="agent-schedules">
    <n-alert v-if="error" type="error" :bordered="false">{{ error }}</n-alert>
    <n-card size="small" title="执行能力" class="agent-capabilities">
      <div class="capability-row"><div><strong>Agent 模型</strong><p>{{ capability?.model_configured ? '已配置执行模型；实际调用结果以执行记录为准' : '尚未配置，请前往模型服务设置 Agent 执行模型' }}</p></div><n-tag size="small" :bordered="false" :type="capability?.model_configured ? 'success' : 'warning'">{{ capability?.model_configured ? '已配置' : '未配置' }}</n-tag></div>
      <div class="capability-row"><div><strong>Bash 沙箱</strong><p>{{ capability?.bash?.message || '正在查询执行环境' }}</p></div><n-tag size="small" :bordered="false" :type="capability?.bash?.state === 'ready' ? 'success' : 'default'">{{ bashLabel(capability?.bash?.state) }}</n-tag></div>
      <div class="capability-actions"><router-link to="/params">沙箱设置</router-link><router-link to="/models">模型服务</router-link><n-button size="small" secondary :loading="checking" @click="checkBash">检查 Bash 环境</n-button></div>
      <n-alert v-if="probe" :type="probe.ok ? 'success' : 'warning'" :bordered="false">{{ probe.message }}</n-alert>
    </n-card>
    <div class="agent-grid">
      <n-card title="新建云团任务" size="small">
        <n-form label-placement="top" :show-feedback="false" :disabled="saving" @submit.prevent="create">
          <n-form-item label="任务名称"><n-input v-model:value="draft.title" maxlength="100" :input-props="{ 'aria-label': '云团任务名称' }" placeholder="例如：每日资料检查" /></n-form-item>
          <n-form-item label="让云团做什么"><n-input v-model:value="draft.goal" type="textarea" maxlength="4000" :autosize="{minRows:4,maxRows:10}" :input-props="{ 'aria-label': '任务要求' }" placeholder="说明目标、需要处理的资料和希望得到的结果" /></n-form-item>
          <n-form-item label="允许使用的工具"><n-select v-model:value="draft.allowed_tools" multiple :options="tools" aria-label="允许使用的工具" placeholder="选择本任务可以调用的工具" /></n-form-item>
          <p class="agent-note">只展示只读工具与沙箱工作区工具。工具还需允许定时任务会话；任务不会获得未选工具的权限。</p>
          <n-form-item label="每日执行时间（北京时间）"><input v-model="draft.daily_time" type="time" aria-label="云团每日执行时间" required :disabled="saving" /></n-form-item>
          <div class="budget-fields"><n-form-item label="最多执行轮数"><n-input-number v-model:value="draft.max_calls" :min="1" :max="10" :precision="0" aria-label="最多执行轮数" /></n-form-item><n-form-item label="任务时限（秒）"><n-input-number v-model:value="draft.timeout_secs" :min="30" :max="900" :precision="0" aria-label="任务时限" /></n-form-item></div>
          <p class="agent-note">到点后进入任务队列，浏览器关闭不影响执行。结果保存在后台，本版不自动推送 QQ。暂停计划不会取消已经启动的任务。</p>
          <n-button attr-type="submit" type="primary" :loading="saving" :disabled="!canCreate">创建云团任务</n-button>
        </n-form>
      </n-card>
      <n-card title="云团任务计划" size="small">
        <template #header-extra><n-button size="small" text :loading="loading" @click="load">刷新计划</n-button></template>
        <EmptyState v-if="!loading && !tasks.length" title="暂无云团任务" />
        <article v-for="task in tasks" :key="task.id" class="agent-job">
          <div class="job-head"><strong>{{ task.title }}</strong><n-tag size="small" :bordered="false" :type="task.enabled ? 'success' : 'default'">{{ task.enabled ? '已启用' : '已暂停' }}</n-tag></div>
          <p class="job-goal">{{ task.goal }}</p>
          <div class="job-tools"><n-tag v-for="tool in task.allowed_tools" :key="tool" size="small" :bordered="false">{{ tool === 'sandbox_exec' ? 'Bash 沙箱' : tool }}</n-tag></div>
          <p class="agent-note">每日 {{ taskTime(task) }} · {{ task.max_calls }} 轮 · {{ task.timeout_secs }} 秒<template v-if="task.enabled"> · 下次 {{ time(task.next_run_at) }}</template></p>
          <div v-if="task.triggered_at" class="job-result"><span>{{ time(task.triggered_at) }} · {{ phaseLabel(task.phase, task.error) }}</span><p v-if="task.error" class="job-error">{{ task.error }}</p><p v-if="task.result">{{ task.result }}</p><router-link v-if="task.task_id" :to="{path:'/tasks',query:{task:task.task_id}}">查看完整执行过程</router-link></div>
          <div v-if="editing?.id === task.id" class="job-actions"><input v-model="editing.time" type="time" aria-label="云团新的执行时间" /><n-button size="small" :disabled="busy.has(task.id)" @click="saveTime(task)">保存时间</n-button><n-button size="small" text @click="editing = null">取消</n-button></div>
          <div v-else class="job-actions">
            <n-button size="small" secondary :loading="busy.has(task.id)" :disabled="busy.has(task.id) || !task.enabled || active(task.phase)" @click="mutate(task,'/run')">立即执行</n-button>
            <n-button size="small" text :disabled="busy.has(task.id)" @click="editing = {id:task.id,time:taskTime(task)}">更改时间</n-button>
            <n-button size="small" text :disabled="busy.has(task.id)" @click="mutate(task,'',{enabled:!task.enabled})">{{ task.enabled ? '暂停计划' : '启用计划' }}</n-button>
            <n-button size="small" text @click="history(task)">执行记录</n-button>
            <n-popconfirm @positive-click="remove(task)"><template #trigger><n-button size="small" text type="error" :disabled="busy.has(task.id)">删除计划</n-button></template>删除计划后保留已创建任务，正在执行的任务需在任务回放中取消。</n-popconfirm>
          </div>
          <div v-if="runs?.id === task.id" class="run-list"><p v-if="!runs.items.length">暂无执行记录</p><article v-for="run in runs.items" :key="run.id"><span>{{ time(run.triggered_at) }} · {{ phaseLabel(run.phase,run.error) }}</span><p v-if="run.error" class="job-error">{{ run.error }}</p><router-link v-if="run.task_id" :to="{path:'/tasks',query:{task:run.task_id}}">查看任务</router-link></article></div>
        </article>
      </n-card>
    </div>
  </section>
</template>

<script setup>
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue'
import { useMessage } from 'naive-ui'
import { api } from '../api'
import EmptyState from './EmptyState.vue'
const message = useMessage()
const capability = ref(null), probe = ref(null), tasks = ref([]), runs = ref(null), editing = ref(null)
const loading = ref(false), saving = ref(false), checking = ref(false), error = ref('')
const busy = reactive(new Set())
const draft = reactive({title:'',goal:'',daily_time:'09:00',allowed_tools:[],max_calls:10,timeout_secs:900})
const tools = computed(() => (capability.value?.tools || []).map(t => ({value:t.name,label:`${t.name === 'sandbox_exec' ? 'Bash 沙箱' : t.name}${t.available ? '' : `（${t.reason}）`}`,disabled:!t.available})))
const canCreate = computed(() => capability.value?.model_configured && draft.title.trim() && draft.goal.trim() && draft.allowed_tools.length && /^(?:[01]\d|2[0-3]):[0-5]\d$/.test(draft.daily_time) && !saving.value)
const taskTime = t => `${String(t.local_hour).padStart(2,'0')}:${String(t.local_minute).padStart(2,'0')}`
const time = ts => new Date(ts*1000).toLocaleString('zh-CN',{timeZone:'Asia/Shanghai',hour12:false})
const active = phase => ['queued','executing','waiting_input','waiting_children','paused'].includes(phase)
const phaseLabel = (phase,error) => ({queued:'已触发，排队中',executing:'执行中',waiting_input:'等待补充，请查看任务回放',waiting_children:'等待子任务',paused:'任务已暂停',finished:'执行完成',failed:'执行失败',cancelled:'已取消'})[phase] || (error ? '未能启动' : '等待执行')
const bashLabel = state => ({disabled:'未启用',unavailable:'环境不可用',restricted:'权限受限',ready:'已注册'})[state] || '查询中'
let stopped = false, timer, controller
async function load() {
  if (loading.value || stopped) return
  loading.value = true; controller = new AbortController()
  try {
    const [caps,jobs] = await Promise.all([api.get('/runtime/agent',{signal:controller.signal}),api.get('/agent-schedules',{signal:controller.signal})])
    if (stopped) return
    capability.value = caps.data; tasks.value = jobs.data.tasks || []
  } catch(e) { if (!stopped) error.value = e.response?.data?.error || '无法读取云团定时任务' }
  finally { loading.value = false }
}
async function checkBash() {
  checking.value = true
  try { probe.value = (await api.post('/runtime/bash/check')).data }
  catch(e) { error.value = e.response?.data?.error || '环境检查失败' }
  finally { checking.value = false }
}
async function create() {
  if (!canCreate.value) return
  saving.value = true; error.value = ''
  try { await api.post('/agent-schedules',{...draft,allowed_tools:[...draft.allowed_tools]}); draft.title=''; draft.goal=''; await load(); message.success('云团任务计划已创建') }
  catch(e) { error.value=e.response?.data?.error || '创建失败' }
  finally { saving.value=false }
}
async function mutate(task,suffix,body) {
  if (busy.has(task.id)) return
  busy.add(task.id); error.value=''
  try { const {data}=await api.post(`/agent-schedules/${task.id}${suffix}`,body); if (data.message) message.info(data.message); await load() }
  catch(e) { error.value=e.response?.data?.error || '操作失败'; await load() }
  finally { busy.delete(task.id) }
}
async function saveTime(task) { await mutate(task,'/time',{daily_time:editing.value.time}); if (!error.value) editing.value=null }
async function remove(task) { if(busy.has(task.id))return; busy.add(task.id); try {await api.delete(`/agent-schedules/${task.id}`); await load()}catch(e){error.value=e.response?.data?.error || '删除失败'}finally{busy.delete(task.id)} }
async function history(task) { try { runs.value={id:task.id,items:(await api.get(`/agent-schedules/${task.id}/history`)).data.runs || []} } catch(e){ error.value=e.response?.data?.error || '执行记录读取失败' } }
onMounted(()=>{void load(); timer=setInterval(()=>{if(!document.hidden && !saving.value && !busy.size)void load()},10000)})
onUnmounted(()=>{stopped=true; clearInterval(timer); controller?.abort()})
</script>

<style scoped>
.agent-schedules {display:grid;gap:16px;min-width:0}
.agent-grid {display:grid;grid-template-columns:minmax(280px,360px) minmax(0,1fr);gap:16px;align-items:start}
.agent-grid > * {min-width:0}
.agent-grid :deep(.n-form-item) {margin-bottom:12px}
.capability-row,.job-head {display:flex;align-items:center;justify-content:space-between;gap:16px}
.capability-row + .capability-row {border-top:1px solid var(--yt-card-border);padding-top:12px;margin-top:12px}
.capability-row p,.agent-note {color:var(--yt-ink-3);font-size:12px;line-height:1.7;margin:6px 0 12px}
.capability-actions,.job-actions,.job-tools {display:flex;gap:10px;align-items:center;flex-wrap:wrap}
.capability-actions {margin:8px 0 12px}
a {color:var(--yt-primary);font-size:13px;text-decoration:none}
.budget-fields {display:grid;grid-template-columns:1fr 1fr;gap:12px}
input[type=time] {padding:7px 10px;border:1px solid var(--yt-card-border);border-radius:6px;color:var(--yt-ink-1);background:var(--yt-surface);font:inherit;color-scheme:light}
:global(html.dark .agent-schedules input[type=time]) {color-scheme:dark}
.agent-job {padding:16px 0;border-bottom:1px solid var(--yt-card-border)}
.agent-job:first-of-type {padding-top:0}.agent-job:last-of-type {border-bottom:0}
.job-goal,.job-result p {white-space:pre-wrap;overflow-wrap:anywhere;font-size:13px;line-height:1.7}
.job-result {background:var(--yt-surface);border:1px solid var(--yt-card-border);border-radius:8px;padding:12px;margin:12px 0;font-size:13px}
.job-error {color:var(--yt-danger);overflow-wrap:anywhere}
.run-list {border-top:1px solid var(--yt-card-border);margin-top:12px;font-size:12px}.run-list article {padding:10px 0;display:grid;gap:4px}
@media(max-width:810px){.agent-grid{grid-template-columns:minmax(0,1fr)}.capability-row{align-items:flex-start}}
</style>
