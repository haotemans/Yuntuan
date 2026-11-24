<template>
  <div class="yt-page plugin-settings">
    <div class="topbar">
      <n-button text @click="router.push('/plugins')" aria-label="返回插件列表">〈 返回插件</n-button>
    </div>
    <n-alert v-if="loadError" type="error" :bordered="false">{{ loadError }}
      <n-button text @click="load">重试</n-button>
    </n-alert>
    <n-skeleton v-if="loading" height="360px" />
    <template v-else-if="detail">
      <header class="yt-page-head plugin-heading">
        <div>
          <h1 class="yt-page-title">{{ detail.display_name || detail.name }}</h1>
          <p>{{ detail.presentation?.purpose || detail.description }}</p>
        </div>
        <div class="heading-right">
          <n-tag size="small" :type="stateTag" :bordered="false">{{ detailStatus.label }}</n-tag>
          <n-switch :value="detail.enabled" :loading="busy || detail.updating" :disabled="busy || authBusy || formDirty || detail.updating || detail.pending_restart" aria-label="启用插件"
            @update:value="toggle" />
        </div>
      </header>
      <n-alert v-if="detail.error" type="warning" :bordered="false">{{ detail.error }}</n-alert>
      <n-alert v-if="detail.pending_reload" type="warning" :bordered="false">新配置尚未正常生效，请修正后重试。</n-alert>
      <n-alert v-if="detail.pending_restart" type="error" :bordered="false">插件无法安全停止，需要重启云团服务。</n-alert>
      <n-alert v-if="actionError" type="error" :bordered="false" closable @close="actionError = ''">{{ actionError }}</n-alert>
      <n-tabs v-model:value="activeTab" type="line" animated>
        <n-tab-pane name="overview" tab="概览">
          <div class="section-grid">
            <section class="panel">
              <h2>命令入口</h2>
              <div class="examples"><code v-for="command in commands" :key="command">{{ command }}</code><span v-if="!commands.length">启用插件后显示可用命令</span></div>
              <template v-if="detail.presentation?.examples?.length">
                <div class="small-label">用法示例</div>
                <div class="examples"><span v-for="example in detail.presentation.examples" :key="example">{{ example }}</span></div>
              </template>
            </section>
            <section class="panel">
              <h2>{{ isConnector ? '连接与授权' : '使用设置' }}</h2>
              <p v-if="connectorGuide">{{ connectorGuide.help }}</p>
              <p v-else-if="detail.requires_onebot">通过 OneBot 执行，实际调用受当前 QQ 账号和所在会话的权限约束。</p>
              <div class="overview-actions">
                <n-button size="small" secondary @click="activeTab='config'">{{ isConnector ? '前往连接账号' : '前往配置' }}</n-button>
                <n-button size="small" secondary @click="activeTab='skills'">查看能力与授权</n-button>
                <a v-if="connectorGuide?.url" :href="connectorGuide.url" target="_blank" rel="noopener noreferrer" class="official-link">查看官方授权及安装说明 ↗</a>
              </div>
            </section>
          </div>
        </n-tab-pane>
        <n-tab-pane name="config" tab="连接配置">
          <ConnectorAuth v-if="isConnector" :key="detail.name" :name="detail.name" :disabled="formDirty || busy || detail.updating || detail.pending_restart" @connected="load(true)" @busy="authBusy=$event" />
          <div v-if="isConnector" class="yt-toolbar connector-actions">
            <n-button size="small" secondary :loading="checkingConnection" :disabled="!canCheckConnection" @click="checkConnection">检查连接</n-button>
            <n-button size="small" text @click="showAdvanced=!showAdvanced">{{ showAdvanced ? '收起高级配置' : '高级配置与调用限制' }}</n-button>
          </div>
          <n-alert v-if="connectionResult" :type="connectionFailed ? 'error' : 'success'" :bordered="false" class="connection-result">{{ connectionResult }}</n-alert>
          <section v-show="!isConnector || showAdvanced" class="panel">
            <div class="config-heading"><h2>插件设置</h2><span v-if="formDirty" class="unsaved">未保存</span></div>
            <n-form label-placement="top" :show-feedback="false" :disabled="busy || authBusy || detail.pending_restart" class="settings-form">
              <n-form-item v-for="field in detail.config_fields || []" :key="field.key" :label="field.label">
                <n-select v-if="field.kind === 'choice'" v-model:value="configDraft[field.key]" :options="field.options || []" />
                <n-switch v-else-if="field.kind === 'boolean'" v-model:value="configDraft[field.key]" :aria-label="field.label" />
                <n-input-number v-else-if="field.kind === 'integer'" v-model:value="configDraft[field.key]" :precision="0" />
                <div v-else class="field-input">
                  <n-input v-model:value="configDraft[field.key]" :type="field.secret ? 'password' : 'text'"
                    :placeholder="field.secret ? (field.has_value ? '已保存，留空保持不变' : '尚未配置') : ''"
                    :autocomplete="field.secret ? 'off' : 'on'" :input-props="{'aria-label':field.label}" />
                  <div v-if="field.secret" class="field-help">
                    {{ field.has_value ? '密钥已保存（不显示明文）' : '尚未设置密钥' }}
                    <n-checkbox v-if="field.has_value" v-model:checked="clearSecrets[field.key]">清除密钥</n-checkbox>
                  </div>
                </div>
              </n-form-item>
              <n-collapse>
                <n-collapse-item title="调用限制及适用会话" name="advanced">
                  <n-form-item label="超时（秒）"><n-input-number v-model:value="timeoutSeconds" :min="0.1" :max="360" /></n-form-item>
                  <n-form-item label="最大并发数"><n-input-number v-model:value="limits.max_concurrency" :min="1" :max="16" :precision="0" /></n-form-item>
                  <n-form-item label="最大文字结果（KiB）"><n-input-number v-model:value="outputKiB" :min="1" :max="1024" /></n-form-item>
                  <n-form-item label="会话范围"><n-select v-model:value="scopeMode" :options="[{label:'全部会话',value:'all'},{label:'仅指定会话',value:'selected'}]" /></n-form-item>
                  <n-input v-if="scopeMode === 'selected'" v-model:value="scopeText" type="textarea" placeholder="每行填写一个会话 ID" />
                </n-collapse-item>
              </n-collapse>
            </n-form>
            <div class="settings-actions">
              <span aria-live="polite">{{ saveFeedback }}</span>
              <n-button :disabled="busy || authBusy || detail.updating || !formDirty" @click="resetDraft(detail)">撤销修改</n-button>
              <n-button type="primary" :loading="busy" :disabled="busy || authBusy || detail.updating || detail.pending_restart || !formDirty" @click="saveConfig">保存并应用</n-button>
            </div>
          </section>
        </n-tab-pane>
        <n-tab-pane name="skills" tab="Skills / Tools">
          <PluginPreflight :key="detail.name" :plugin="detail" :capabilities="capabilities" />
          <section class="panel">
            <h2>已注册能力</h2><p>管理技能与工具的启用状态和适用范围。</p>
            <div v-for="skill in detail.skills || []" :key="skill.name" class="capability">
              <div><code>{{ skill.name }}</code><p>{{ skill.description }}</p><small>{{ (skill.commands || []).join(' · ') || '自然语言调用' }}</small></div>
              <div class="capability-actions">
                <n-button v-if="capability(skill.name,'skill')" size="tiny" secondary :disabled="busy || capabilityBusy"
                  :aria-label="`编辑技能 ${skill.name} 的会话范围`" @click="editCapabilityScope(skill.name,'skill')">{{ scopeLabel(skill.name,'skill') }}</n-button>
                <n-switch :value="capabilityEnabled(skill.name,'skill')" :disabled="busy || capabilityBusy || !capability(skill.name,'skill')" :aria-label="`技能 ${skill.name}`" @update:value="value => toggleCapability(skill.name,'skill',value)" />
              </div>
            </div>
            <div v-for="tool in detail.tools || []" :key="tool.name" class="capability">
              <div><code>{{ tool.name }}</code><p>{{ capability(tool.name,'tool')?.description || tool.description }}</p><small>Tool · 累计调用 {{ capability(tool.name,'tool')?.calls || 0 }} 次</small>
                <details v-if="capability(tool.name,'tool')?.tool_contract" class="tool-contract">
                  <summary>调用参数</summary>
                  <pre class="tool-schema">{{ JSON.stringify(capability(tool.name,'tool').tool_contract.input_schema,null,2) }}</pre>
                  <small>{{ capability(tool.name,'tool').tool_contract.policy?.effect || 'unknown' }} · {{ capability(tool.name,'tool').tool_contract.policy?.timeout_ms || 0 }} ms</small>
                </details>
              </div>
              <div class="capability-actions">
                <n-button v-if="capability(tool.name,'tool')" size="tiny" secondary :disabled="busy || capabilityBusy"
                  :aria-label="`编辑工具 ${tool.name} 的会话范围`" @click="editCapabilityScope(tool.name,'tool')">{{ scopeLabel(tool.name,'tool') }}</n-button>
                <n-switch :value="capabilityEnabled(tool.name,'tool')" :disabled="busy || capabilityBusy || !capability(tool.name,'tool')" :aria-label="`工具 ${tool.name}`" @update:value="value => toggleCapability(tool.name,'tool',value)" />
              </div>
            </div>
            <p v-if="!detail.skills?.length && !detail.tools?.length">启用插件并完成配置后，将在这里看到工具和 Skill。</p>
          </section>
        </n-tab-pane>
        <n-tab-pane v-if="detail.name === 'quark-drive'" name="links" tab="分享守护">
          <section class="panel">
            <h2>分享链接每日检查</h2>
            <p>默认每天北京时间 09:00，通过夸克官方 CLI 检查已登记的分享 URL。此功能只做只读检测、保存结果，不会在平台限制或链接异常时自动换链。网络失败不等于违规封禁。</p>
            <div class="watch-form">
              <n-input v-model:value="watchTitle" placeholder="任务名称（可选）" aria-label="守护任务名称" />
              <n-input v-model:value="watchUrl" placeholder="https://pan.quark.cn/s/..." aria-label="夸克分享链接" />
              <n-button type="primary" :loading="watchBusy" :disabled="watchBusy || !watchUrl.trim()" @click="addWatch">添加每日检查</n-button>
            </div>
            <n-alert v-if="watchError" type="error" :bordered="false" closable @close="watchError = ''">{{ watchError }}</n-alert>
            <div v-if="!watches.length" class="watch-empty">尚未登记分享链接。先在「连接配置」启用并授权夸克插件，然后添加链接。</div>
            <div v-for="item in watches" :key="item.id" class="watch-item">
              <div class="watch-info">
                <strong>{{ item.title }}</strong>
                <a :href="item.resource_key" target="_blank" rel="noopener noreferrer">{{ item.resource_key }}</a>
                <small>状态：{{ watchStatus(item.last_status) }} · 下次检查：{{ fmtTime(item.next_run_at) }}</small>
                <small v-if="item.last_run_at">最近检查：{{ fmtTime(item.last_run_at) }}</small>
                <small v-if="item.last_summary">{{ item.last_summary }}</small>
              </div>
              <div class="watch-actions">
                <n-switch :value="item.enabled" :disabled="watchBusy" :aria-label="`启用守护 ${item.id}`" @update:value="enabled => toggleWatch(item,enabled)" />
                <n-button size="small" :loading="watchBusy && watchBusyId===item.id" :disabled="watchBusy || !item.enabled" @click="checkWatch(item)">立即检查</n-button>
                <n-button size="small" tertiary type="error" :disabled="watchBusy" @click="removeWatch(item)">删除</n-button>
              </div>
            </div>
            <p class="watch-caution">需要官方 CLI 已登录；定时检查复用云团已有 Tool 权限和插件调用统计。检查失败只记录异常，不会自动重新分享。</p>
          </section>
        </n-tab-pane>
        <n-tab-pane name="runtime" tab="运行记录">
          <section class="panel">
            <h2>调用状态</h2>
            <p>本次服务启动后的调用统计。</p>
            <dl class="stats">
              <div><dt>执行中</dt><dd>{{ detail.invocations?.active || 0 }}</dd></div>
              <div><dt>成功</dt><dd>{{ detail.invocations?.succeeded || 0 }}</dd></div>
              <div><dt>失败</dt><dd>{{ detail.invocations?.failed || 0 }}</dd></div>
              <div><dt>最近耗时</dt><dd>{{ pluginDuration(detail.invocations?.last_duration_ms) }}</dd></div>
            </dl>
            <n-alert v-if="detail.invocations?.last_error" type="warning" :bordered="false">{{ detail.invocations.last_error }}</n-alert>
          </section>
          <PluginHistory :key="detail.name" :name="detail.name" />
        </n-tab-pane>
      </n-tabs>
      <n-modal :show="!!scopeEditor" preset="card" title="编辑会话范围" class="capability-scope-dialog"
        :style="{ width: 'min(480px, calc(100vw - 32px))' }" :closable="!capabilityBusy"
        :mask-closable="!capabilityBusy" :close-on-esc="!capabilityBusy" @update:show="closeCapabilityScope">
        <template v-if="scopeEditor">
          <p><code>{{ scopeEditor.name }}</code> · {{ scopeEditor.kind === 'tool' ? 'Tool' : 'Skill' }}</p>
          <p>此范围与插件设置、账号权限共同生效。</p>
          <n-form :disabled="capabilityBusy" :show-feedback="false">
            <n-form-item label="允许调用的会话">
              <n-select v-model:value="scopeEditor.mode" aria-label="允许调用的会话"
                :options="[{label:'全部会话',value:'all'},{label:'仅指定会话',value:'selected'}]" />
            </n-form-item>
            <n-form-item v-if="scopeEditor.mode === 'selected'" label="会话 ID">
              <n-input v-model:value="scopeEditor.text" type="textarea"
                :input-props="{'aria-label':'会话 ID'}" placeholder="每行填写一项，如 group:123456 或 private:dm_123456" />
            </n-form-item>
          </n-form>
          <n-alert v-if="scopeEditor.error" type="error" :bordered="false">{{ scopeEditor.error }}</n-alert>
          <div class="settings-actions">
            <n-button :disabled="capabilityBusy" @click="closeCapabilityScope(false)">取消</n-button>
            <n-button type="primary" :loading="capabilityBusy" @click="saveCapabilityScope">保存范围</n-button>
          </div>
        </template>
      </n-modal>
    </template>
    <EmptyState v-else-if="!loadError" title="该插件不存在或未编译安装" />
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useMessage } from 'naive-ui'
import { api } from '../api'
import { pluginDuration, pluginStatus } from '../plugin-ui'
import EmptyState from '../components/EmptyState.vue'
import ConnectorAuth from '../components/ConnectorAuth.vue'
import PluginHistory from '../components/PluginHistory.vue'
import PluginPreflight from '../components/PluginPreflight.vue'

const route = useRoute()
const router = useRouter()
const message = useMessage()
const plugins = ref([])
const capabilities = ref([])
const loading = ref(true)
const busy = ref(false)
const loadError = ref('')
const actionError = ref('')
const saved = ref('')
const checkingConnection = ref(false)
const connectionResult = ref('')
const connectionFailed = ref(false)
const authBusy = ref(false)
const showAdvanced = ref(false)
const connectorGuides = {
  'tencent-docs': {
    help: '打开官方授权页取得个人 Token，回填后由云团验证并保存连接。',
    url: 'https://docs.qq.com/open/auth/mcp.html',
  },
  'quark-drive': {
    help: '发起账号授权，在官方页面登录后回填授权结果。',
    url: 'https://github.com/quark-clouddrive/quarkclouddrive_offical',
  },
  qzone: { help: '使用 NapCat 当前登录的 QQ 账号，无需另填 Cookie。发布由超级管理员授权。' },
}
const connectorGuide = computed(() => connectorGuides[detail.value?.name])
const isConnector = computed(() => !!connectorGuide.value?.url)
const canCheckConnection = computed(() => !!detail.value?.enabled && detail.value.loaded &&
  !detail.value.error && !detail.value.updating && !detail.value.pending_restart && !detail.value.pending_reload &&
  !busy.value && !authBusy.value && !checkingConnection.value && !formDirty.value)
async function checkConnection() {
  if (!canCheckConnection.value) return
  const name = detail.value.name
  checkingConnection.value = true
  connectionResult.value = ''
  try {
    const {data} = await api.post('/plugins/connection/check',{name})
    if (detail.value?.name !== name) return
    connectionResult.value = data.summary || '连接检查已完成'
    connectionFailed.value = false
  } catch(e) {
    if (detail.value?.name !== name) return
    connectionResult.value = e.response?.data?.error || '连接失败，请检查安装、授权、网络及调用范围'
    connectionFailed.value = true
  } finally { checkingConnection.value = false }
}

const watches = ref([])
const watchTitle = ref('')
const watchUrl = ref('')
const watchBusy = ref(false)
const watchBusyId = ref(null)
const watchError = ref('')
const activeTab = computed({
  get() {
    const tab = typeof route.query.tab === 'string' ? route.query.tab : ''
    if (tab === 'links' && route.params.name === 'quark-drive') return tab
    return ['overview','config','skills','runtime'].includes(tab) ? tab : 'overview'
  },
  set(tab) { void router.replace({ query: { ...route.query, tab } }) },
})
const detail = computed(() => plugins.value.find(p => p.name === route.params.name))
const detailStatus = computed(() => pluginStatus(detail.value || {}, busy.value))
const stateTag = computed(() => ({ success:'success', error:'error', warning:'warning', working:'info' })[detailStatus.value.tone] || 'default')
const watchLabels = { pending:'待首次检查', checking:'检查中', accessible:'链接可访问', check_failed:'检查失败（需核实）' }
const watchStatus = status => watchLabels[status] || '状态未知'
const fmtTime = stamp => stamp ? new Date(stamp * 1000).toLocaleString('zh-CN', {timeZone:'Asia/Shanghai',hour12:false}) : '尚未安排'
async function loadWatches() {
  if (detail.value?.name !== 'quark-drive') return
  try {
    const {data} = await api.get('/plugins/quark/watches')
    watches.value = data.watches || []
  } catch(e) { watchError.value = e.response?.data?.error || '无法获取分享守护列表' }
}
async function addWatch() {
  if (watchBusy.value) return
  watchBusy.value = true
  watchError.value = ''
  try {
    await api.post('/plugins/quark/watches',{url:watchUrl.value,title:watchTitle.value})
    watchUrl.value = ''
    watchTitle.value = ''
    message.success('已登记每日检查任务')
    await loadWatches()
  } catch(e) { watchError.value = e.response?.data?.error || '添加失败，请检查授权与链接' }
  finally { watchBusy.value = false }
}
async function checkWatch(item) {
  if (watchBusy.value) return
  watchBusy.value = true
  watchBusyId.value = item.id
  watchError.value = ''
  try {
    await api.post(`/plugins/quark/watches/${item.id}/check`,{})
    await loadWatches()
    message.success('检查结果已更新')
  } catch(e) { watchError.value = e.response?.data?.error || '检查失败' }
  finally { watchBusy.value = false; watchBusyId.value = null }
}
async function toggleWatch(item,enabled) {
  if (watchBusy.value) return
  watchBusy.value = true
  try {
    await api.post(`/plugins/quark/watches/${item.id}`,{enabled})
    await loadWatches()
  } catch(e) { watchError.value = e.response?.data?.error || '更改失败' }
  finally { watchBusy.value = false }
}
async function removeWatch(item) {
  if (watchBusy.value || !window.confirm('删除这条分享守护任务？不会删除网盘中的文件。')) return
  watchBusy.value = true
  try {
    await api.delete(`/plugins/quark/watches/${item.id}`)
    await loadWatches()
  } catch(e) { watchError.value = e.response?.data?.error || '删除失败' }
  finally { watchBusy.value = false }
}
const commands = computed(() => [...new Set((detail.value?.skills || []).flatMap(s => s.commands || []))])
const configDraft = ref({})
const clearSecrets = ref({})
const limits = ref({timeout_ms:30000,max_concurrency:1,max_output_bytes:65536})
const scopeMode = ref('all')
const scopeText = ref('')
const baseline = ref('')
const signature = () => JSON.stringify([configDraft.value,clearSecrets.value,limits.value,scopeMode.value,scopeText.value])
const formDirty = computed(() => !!baseline.value && signature() !== baseline.value)
const saveFeedback = computed(() => {
  if (!saved.value || formDirty.value || !detail.value) return ''
  if (busy.value || detail.value.updating) return '已提交配置，正在后台应用'
  if (loadError.value) return '配置已提交，等待重新读取状态'
  if (detail.value.error || detail.value.pending_reload || detail.value.pending_restart) return '请查看插件当前状态'
  return '配置已应用'
})
const timeoutSeconds = computed({get:()=>limits.value.timeout_ms/1000,set:value=>{limits.value.timeout_ms=value==null?0:Math.round(value*1000)}})
const outputKiB = computed({get:()=>limits.value.max_output_bytes/1024,set:value=>{limits.value.max_output_bytes=value==null?0:Math.round(value*1024)}})
function capability(name,kind) { return capabilities.value.find(c => c.name===name && c.kind===kind) }
function capabilityEnabled(name,kind) { return capability(name,kind)?.enabled ?? true }
const capabilityBusy = ref(false)
const scopeEditor = ref(null)
let capabilityRevision = 0
function scopeLabel(name,kind) {
  const count = capability(name,kind)?.scopes?.length || 0
  return count ? `${count} 个会话` : '全部会话'
}
function editCapabilityScope(name,kind) {
  const item = capability(name,kind)
  if (!item || capabilityBusy.value) return
  const scopes = item.scopes || []
  scopeEditor.value = {name, kind, mode:scopes.length ? 'selected' : 'all', text:scopes.join('\n'), error:''}
}
function closeCapabilityScope(show) {
  if (!show && !capabilityBusy.value) scopeEditor.value = null
}
async function saveCapabilityScope() {
  const editor = scopeEditor.value
  if (!editor || capabilityBusy.value) return
  editor.error = ''
  const scopes = editor.mode === 'all' ? [] : [...new Set(editor.text.split(/\r?\n/).map(line => line.trim()).filter(Boolean))]
  if (editor.mode === 'selected' && !scopes.length) {
    editor.error = '请填写至少一个会话，或选择全部会话。'
    return
  }
  const saved = await updateCapability(editor.name, editor.kind, {scopes}, editor)
  if (saved && scopeEditor.value === editor) scopeEditor.value = null
}
function toggleCapability(name,kind,enabled) {
  return updateCapability(name,kind,{enabled})
}
async function updateCapability(name,kind,change,editor = null) {
  if (!capability(name,kind) || busy.value || capabilityBusy.value) return false
  const plugin = route.params.name
  capabilityBusy.value = true
  actionError.value = ''
  try {
    const {data} = await api.patch('/capabilities',{name,kind,...change})
    capabilityRevision++
    const item = capability(name,kind)
    if (item) Object.assign(item,{enabled:data.enabled,scopes:data.scopes})
    if (!disposed && route.params.name === plugin) message.success(editor ? '已保存会话范围' : '已更新能力开关')
    return true
  } catch(e) {
    const error = e.response?.data?.error || '能力更新失败'
    if (editor) editor.error = error
    else if (route.params.name === plugin) actionError.value = error
    return false
  } finally { capabilityBusy.value = false }
}
function resetDraft(plugin) {
  configDraft.value = Object.fromEntries((plugin.config_fields || []).map(f => [f.key, f.secret ? '' : (plugin.settings?.config?.[f.key] ?? f.default)]))
  clearSecrets.value = {}
  limits.value = {timeout_ms:plugin.settings?.timeout_ms ?? 30000,max_concurrency:plugin.settings?.max_concurrency ?? 1,max_output_bytes:plugin.settings?.max_output_bytes ?? 65536}
  scopeMode.value=plugin.settings?.allowed_chats == null ? 'all' : 'selected'
  scopeText.value=(plugin.settings?.allowed_chats || []).join('\n')
  baseline.value=signature()
}
let disposed=false
let timer
let fetching=false
let loadRevision = 0
async function load(force = false) {
  if (disposed || (fetching && force !== true)) return
  const revision = ++loadRevision
  fetching=true
  const loadedCapabilityRevision = capabilityRevision
  try {
    const [pluginsResponse, capResponse] = await Promise.all([api.get('/plugins/list'),api.get('/capabilities')])
    if (disposed || revision !== loadRevision) return
    plugins.value=pluginsResponse.data.plugins || []
    if (loadedCapabilityRevision === capabilityRevision) capabilities.value=capResponse.data.capabilities || []
    if (detail.value && (!baseline.value || !formDirty.value)) resetDraft(detail.value)
    loadError.value=''
    if (activeTab.value === 'links' && !watchBusy.value) await loadWatches()
  } catch(e) {
    if (!disposed && revision === loadRevision) loadError.value=e.response?.data?.error || '无法获取插件详情'
  } finally {
    if (revision === loadRevision) { loading.value=false; fetching=false }
  }
}
watch(() => route.params.name, () => {
  baseline.value='';saved.value='';actionError.value='';connectionResult.value='';showAdvanced.value=false;authBusy.value=false;scopeEditor.value=null
  void load(true)
})
watch(activeTab, tab => { if (tab === 'links') void loadWatches() })
async function saveConfig() {
  const plugin=detail.value
  if (!plugin || busy.value || authBusy.value || plugin.updating || plugin.pending_restart || !formDirty.value) return
  busy.value=true
  actionError.value=''
  saved.value=''
  try {
    const settings={...plugin.settings,...limits.value,config:{...plugin.settings?.config}}
    for (const field of plugin.config_fields || []) {
      const value=configDraft.value[field.key]
      if (field.secret) {
        delete settings.config[field.key]
        if (clearSecrets.value[field.key]) settings.config[field.key]=''
        else if (value) settings.config[field.key]=value
      } else settings.config[field.key]=value
    }
    settings.allowed_chats=scopeMode.value==='all'?null:[...new Set(scopeText.value.split(/[\s,，]+/).filter(Boolean))]
    await api.post('/plugins/config',{name:plugin.name,settings})
    if (disposed || route.params.name !== plugin.name) return
    saved.value='submitted'
    connectionResult.value=''
    baseline.value=signature()
    await load(true)
  } catch(e) {
    if (route.params.name === plugin.name) actionError.value=e.response?.data?.error || '保存配置失败'
  }
  finally {busy.value=false}
}
async function toggle(enabled) {
  if (!detail.value || busy.value || authBusy.value || formDirty.value || detail.value.updating || detail.value.pending_restart) return
  busy.value=true
  actionError.value=''
  try {
    const response=await api.post('/plugins/toggle',{name:detail.value.name,enabled})
    message.info(response.data.applying?'插件状态正在应用':'插件状态已更新')
    await load(true)
  } catch(e) {actionError.value=e.response?.data?.error || '更改启停状态失败'}
  finally {busy.value=false}
}
async function poll() {
  if (!disposed && !document.hidden && !busy.value && !capabilityBusy.value) await load()
  if (!disposed) timer=setTimeout(poll,2500)
}
onMounted(() => {void poll()})
onUnmounted(() => {disposed=true;clearTimeout(timer)})
</script>

<style scoped>
.connection-result {white-space:pre-wrap;overflow-wrap:anywhere}
.plugin-settings {max-width:1180px;margin:0 auto;padding-bottom:58px}
.topbar {display:flex;align-items:center}
.plugin-heading h1 {margin:0 0 6px}
.plugin-heading p {color:var(--yt-ink-3);font-size:13px;margin:0}
.heading-right {display:flex;align-items:center;gap:14px}
.section-grid {display:grid;grid-template-columns:repeat(2,minmax(0,1fr));align-items:start;gap:16px}
.panel {background:var(--yt-surface);border:1px solid var(--yt-card-border);border-radius:12px;box-shadow:var(--yt-card-shadow);padding:18px;min-width:0;margin:12px 0}
.panel h2 {font-size:16px;color:var(--yt-ink-1);margin:0 0 12px}
.panel p {font-size:13px;line-height:1.8;color:var(--yt-ink-2)}
.small-label {font-size:12px;color:var(--yt-ink-3);margin:22px 0 8px}
.examples {display:flex;flex-wrap:wrap;gap:8px}
.examples code,.examples span {padding:7px 10px;border-radius:8px;background:var(--yt-hover-bg);font-size:12px;overflow-wrap:anywhere}
.overview-actions {display:flex;align-items:center;flex-wrap:wrap;gap:10px}
.official-link {color:var(--yt-primary);font-size:13px}
.connector-actions {margin:12px 0}
.settings-form {max-width:750px;margin-top:22px}
.field-input {width:100%}
.field-help {display:flex;gap:16px;align-items:center;font-size:11px;color:var(--yt-ink-3);margin-top:6px}
.config-heading {display:flex;justify-content:space-between;align-items:start}
.config-heading p {margin:0}
.unsaved {font-size:12px;color:var(--yt-warning)}
.settings-actions {display:flex;justify-content:flex-end;align-items:center;flex-wrap:wrap;gap:10px;margin-top:20px}
.settings-actions span {color:var(--yt-ink-3);font-size:12px;margin-right:auto}
.capability {display:flex;align-items:center;justify-content:space-between;gap:20px;padding:16px 0;border-bottom:1px solid var(--yt-card-border)}
.capability > div:first-child {min-width:0}
.capability-actions {display:flex;align-items:center;gap:10px;flex-shrink:0}
.capability-scope-dialog code {overflow-wrap:anywhere}
.tool-contract {margin-top:10px}.tool-contract summary {cursor:pointer;font-size:12px}.tool-schema {white-space:pre-wrap;overflow-wrap:anywhere;font-size:12px;max-width:100%;max-height:320px;overflow:auto}
.capability code {overflow-wrap:anywhere;color:var(--yt-ink-1)}
.capability p {margin:7px 0}
.capability small {color:var(--yt-ink-3)}
.stats {display:grid;grid-template-columns:repeat(4,1fr);gap:18px}
.stats dt {font-size:12px;color:var(--yt-ink-3)}
.stats dd {font-size:23px;margin:5px 0;font-variant-numeric:tabular-nums}
.watch-form {display:grid;grid-template-columns:minmax(140px,1fr) minmax(220px,2fr) auto;gap:10px;margin:24px 0}
.watch-item {display:flex;align-items:flex-start;justify-content:space-between;gap:20px;padding:18px 0;border-top:1px solid var(--yt-card-border)}
.watch-info {display:flex;flex-direction:column;gap:7px;min-width:0}
.watch-info strong {font-size:14px;color:var(--yt-ink-1)}
.watch-info a {font-size:12px;color:var(--yt-primary);word-break:break-all}
.watch-info small,.watch-caution,.watch-empty {font-size:12px;color:var(--yt-ink-3);line-height:1.6}
.watch-actions {display:flex;align-items:center;gap:10px;flex-shrink:0}
.watch-empty {padding:22px 0}
@media(max-width:760px) { .watch-form {grid-template-columns:1fr} .watch-item {flex-direction:column} .watch-actions {flex-wrap:wrap} }
@media(max-width:760px) {.plugin-heading{align-items:start;flex-direction:column}.section-grid{grid-template-columns:1fr}.stats{grid-template-columns:repeat(2,1fr)}.panel{padding:18px}}
@media(max-width:480px) {.capability{align-items:flex-start;flex-direction:column;gap:12px}}
</style>
