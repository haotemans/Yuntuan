<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <span class="yt-page-title">平台接入</span>
      <n-button size="small" secondary :loading="loading" :disabled="busy" @click="load">刷新</n-button>
    </div>
    <n-alert v-if="loadError" type="error" :bordered="false">{{ loadError }}</n-alert>
    <n-alert v-if="statusError" type="warning" :bordered="false">{{ statusError }}</n-alert>
    <div class="platform-grid">
      <n-card v-for="platform in platforms" :key="platform.id" size="small" :title="platform.title">
        <template #header-extra>
          <n-tag size="small" :type="status(platform.id)?.state === 'online' ? 'success' : status(platform.id)?.state === 'error' ? 'error' : 'default'">
            {{ labels[status(platform.id)?.state] || '状态未知' }}
          </n-tag>
        </template>
        <p class="platform-description">{{ platform.description }}</p>
        <dl class="platform-facts">
          <div>
            <dt>运行账号</dt>
            <dd class="mono">{{ status(platform.id)?.account || '—' }}</dd>
          </div>
          <div v-if="platform.endpoint">
            <dt>{{ platform.endpoint.label }}</dt>
            <dd class="mono">{{ platform.endpoint.format(savedAddresses[platform.id]) }}</dd>
          </div>
        </dl>
        <n-alert v-if="status(platform.id)?.detail" type="warning" :bordered="false" class="platform-detail">
          {{ status(platform.id)?.detail }}
        </n-alert>

        <n-form :show-feedback="false" :disabled="busy || !configReady">
          <n-form-item label="启用接入">
            <n-switch v-model:value="forms[platform.id].enabled" :aria-label="`启用 ${platform.title}`" />
          </n-form-item>
          <n-form-item :label="platform.addressLabel">
            <n-input v-model:value="forms[platform.id].address" :placeholder="platform.addressPlaceholder"
              :input-props="{'aria-label':platform.addressLabel}" />
          </n-form-item>
          <n-form-item :label="platform.secretLabel">
            <n-input v-model:value="forms[platform.id].secret" type="password" show-password-on="click"
              :input-props="{'aria-label':platform.secretLabel,autocomplete:'new-password'}"
              :placeholder="forms[platform.id].hasSecret ? '已设置，留空保留原值' : '尚未设置'" />
          </n-form-item>
        </n-form>
        <a v-if="platform.help" :href="platform.help.url" target="_blank" rel="noopener noreferrer">{{ platform.help.label }} ↗</a>
        <div class="form-actions">
          <n-button type="primary" size="small" :loading="saving === platform.id" :disabled="busy || !configReady" @click="save(platform.id, false)">保存配置</n-button>
          <n-button size="small" secondary :disabled="busy || !configReady" @click="save(platform.id, true)">保存并重启云团</n-button>
        </div>
        <p v-if="feedback[platform.id]" role="status" :class="['save-msg', feedback[platform.id].ok ? 'ok' : 'fail']">{{ feedback[platform.id].message }}</p>
      </n-card>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { api } from '../api'

const platforms = [
  {
    id:'onebot', title:'OneBot 11', description:'通过 NapCat 连接 QQ。',
    configKey:'napcat', addressKey:'listen_addr', secretKey:'token',
    defaultEnabled:true, defaultAddress:'127.0.0.1:6199',
    addressLabel:'监听地址', addressPlaceholder:'127.0.0.1:6199', secretLabel:'连接 Token',
    endpoint:{label:'反向 WebSocket', format:address => `ws://${address}/ws`},
  },
  {
    id:'qq_official', title:'QQ 官方机器人', description:'通过 QQ 开放平台接收私聊和群内 @，支持文本回复。',
    configKey:'qq_official', addressKey:'app_id', secretKey:'app_secret',
    defaultEnabled:false, defaultAddress:'',
    addressLabel:'AppID', addressPlaceholder:'开放平台的机器人 AppID', secretLabel:'AppSecret',
    help:{url:'https://q.qq.com/',label:'QQ 开放平台'},
  },
]
const byId = Object.fromEntries(platforms.map(platform => [platform.id,platform]))
const labels = {online:'已连接', offline:'未连接', disabled:'未启用', connecting:'连接中', reconnecting:'重连中', error:'连接异常'}
const statuses = ref([])
const status = id => statuses.value.find(item => item.id === id)
const forms = ref(Object.fromEntries(platforms.map(platform => [platform.id,{
  enabled:platform.defaultEnabled,address:platform.defaultAddress,secret:'',hasSecret:false,
}])))
const feedback = ref({})
const savedAddresses = ref(Object.fromEntries(platforms.map(platform => [platform.id,platform.defaultAddress])))
const loading = ref(false)
const saving = ref('')
const restarting = ref(false)
const busy = computed(() => !!saving.value || restarting.value)
const configReady = ref(false)
const loadError = ref('')
const statusError = ref('')

async function loadStatuses() {
  try {
    const {data} = await api.get('/platforms')
    statuses.value = data.platforms || []
    statusError.value = ''
  } catch {
    statuses.value = []
    statusError.value = '无法获取运行状态'
  }
}

function readSection(config, id) {
  const platform = byId[id]
  const section = config[platform.configKey] || {}
  forms.value[id] = {
    enabled: section.enabled ?? platform.defaultEnabled,
    address: section[platform.addressKey] ?? platform.defaultAddress,
    secret: '',
    hasSecret: !!section[platform.secretKey],
  }
  savedAddresses.value[id] = forms.value[id].address
}

async function load() {
  if (loading.value) return
  loading.value = true
  const statusRequest = loadStatuses()
  try {
    const {data} = await api.get('/config')
    for (const {id} of platforms) readSection(data.config || {}, id)
    configReady.value = true
    loadError.value = ''
  } catch (e) {
    loadError.value = e.response?.data?.error || '无法读取平台配置'
  } finally {
    await statusRequest
    loading.value = false
  }
}

async function save(id, restart) {
  if (busy.value || !configReady.value) return
  saving.value = id
  delete feedback.value[id]
  const {configKey:key,addressKey,secretKey} = byId[id]
  const form = forms.value[id]
  try {
    const response = await api.get('/config')
    const config = response.data.config || {}
    const section = {...config[key], enabled:form.enabled, [addressKey]:form.address.trim()}
    if (form.secret) section[secretKey] = form.secret.trim()
    const {data} = await api.post('/config', {config:{...config,[key]:section}})
    if (form.secret) section[secretKey] = '***'
    readSection({[key]:section}, id)
    const needsRestart = (data.requires_restart || []).some(item => item.includes(key))
    feedback.value[id] = {ok:true,message:needsRestart ? '已保存，重启云团后生效。' : '配置已保存。'}
  } catch (e) {
    feedback.value[id] = {ok:false,message:e.response?.data?.error || '保存失败'}
    return
  } finally { saving.value = '' }
  if (!restart) return
  restarting.value = true
  try {
    await api.post('/config/restart')
    feedback.value[id] = {ok:true,message:'正在重启云团…'}
    restartTimer = setTimeout(() => window.location.reload(), 3200)
  } catch (e) {
    restarting.value = false
    feedback.value[id] = {ok:false,message:e.response?.data?.error || '配置已保存，重启请求未确认。'}
  }
}

let timer
let restartTimer
onMounted(() => {
  void load()
  timer = setInterval(() => { if (!document.hidden && !busy.value && !loading.value) void loadStatuses() }, 5000)
})
onUnmounted(() => { clearInterval(timer); clearTimeout(restartTimer) })
</script>

<style scoped>
.platform-grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(min(100%,340px),1fr)); gap:16px; align-items:start }
.platform-description,.save-msg { color:var(--yt-ink-2); font-size:13px }
.platform-facts { display:grid; grid-template-columns:repeat(auto-fit,minmax(120px,1fr)); gap:10px; margin:18px 0 }
dt { color:var(--yt-ink-3); font-size:12px }
dd { margin:4px 0 0; overflow-wrap:anywhere }
.platform-detail { margin-bottom:16px }
.form-actions { display:flex; flex-wrap:wrap; gap:10px; margin-top:18px }
.save-msg.ok { color:var(--yt-ok) }
.save-msg.fail { color:var(--yt-danger) }
</style>
