<template>
  <section class="auth-panel">
    <h2>{{ quark ? '连接夸克账号' : '连接腾讯文档' }}</h2>
    <p v-if="quark">云团准备运行组件和配置路径。你只需打开授权页登录，再回填授权结果。</p>
    <p v-else>打开腾讯文档授权页获取个人 Token，回填后由云团检查并保存连接。</p>
    <n-alert v-if="disabled" type="info" :bordered="false">请先保存或撤销下方尚未应用的修改。</n-alert>
    <div class="auth-actions">
      <n-button v-if="quark" type="primary" :loading="working" :disabled="disabled || working || occupied" @click="begin">{{ phase === 'waiting' || phase === 'error' ? '重新获取授权链接' : '开始授权登录' }}</n-button>
      <a v-else href="https://docs.qq.com/open/auth/mcp.html" target="_blank" rel="noopener noreferrer">打开腾讯文档授权页 ↗</a>
      <a v-if="phase === 'waiting' && safeLink" :href="safeLink" target="_blank" rel="noopener noreferrer">打开夸克授权页 ↗</a>
    </div>
    <template v-if="!quark || phase === 'waiting'">
      <label :for="inputId">{{ quark ? '跳转链接或授权码' : '个人 Token' }}</label>
      <n-input v-model:value="value" type="password" autocomplete="off" :input-props="{id:inputId,'aria-label':quark ? '跳转链接或授权码' : '授权页个人 Token'}"
        :disabled="disabled || working || occupied" :placeholder="quark ? '粘贴授权完成后的完整跳转链接或授权码' : '粘贴官方授权页生成的个人 Token'" />
      <p v-if="quark" class="hint">跳转页面无法打开时，也可复制地址栏链接回填。授权码仅使用一次。</p>
      <n-button type="primary" :loading="working" :disabled="disabled || working || occupied || !value.trim()" @click="complete">保存并连接</n-button>
    </template>
    <n-alert v-if="error || status.message" :type="error || phase === 'error' ? 'error' : phase === 'connected' ? 'success' : 'info'" :bordered="false" class="auth-status">{{ error || status.message }}</n-alert>
  </section>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { api } from '../api'
const props = defineProps({name:{type:String,required:true},disabled:Boolean})
const emit = defineEmits(['connected','busy'])
const quark = computed(() => props.name === 'quark-drive')
const inputId = computed(() => `auth-result-${props.name}`)
const value = ref('')
const error = ref('')
const status = ref({phase:'idle',message:'',authorization_url:''})
const submitting = ref(false)
const phase = computed(() => status.value.phase)
const working = computed(() => submitting.value || ['preparing','connecting'].includes(phase.value))
const occupied = computed(() => phase.value === 'occupied')
// Defense in depth: only the backend's official HTTPS Quark authorization link.
const safeLink = computed(() => {
  try {
    const u=new URL(status.value.authorization_url)
    return u.protocol==='https:' && !u.username && !u.password && (!u.port || u.port==='443') &&
      (u.hostname==='quark.cn' || u.hostname.endsWith('.quark.cn')) && u.searchParams.has('client_id') ? u.href : ''
  } catch { return '' }
})
let disposed=false
let timer
async function refresh() {
  try {
    const {data}=await api.get(`/plugins/auth/${props.name}`)
    if (disposed) return
    const connected=data.phase==='connected' && phase.value!=='connected'
    status.value=data
    emit('busy',working.value || occupied.value)
    if (connected) {value.value='';emit('connected')}
  } catch { if (!disposed) error.value='无法读取授权进度，请刷新页面检查' }
}
async function begin() {
  if (working.value || props.disabled) return
  submitting.value=true;error.value='';value.value='';emit('busy',true)
  try {
    const {data}=await api.post(`/plugins/auth/${props.name}`,{})
    status.value={...data,message:'正在准备官方组件及授权链接，请稍候'}
  } catch(e) {error.value=e.response?.data?.error || '无法发起授权，请稍后重试'}
  finally {submitting.value=false;emit('busy',working.value)}
}
async function complete() {
  if (working.value || props.disabled || !value.value.trim()) return
  submitting.value=true;error.value='';emit('busy',true)
  const submitted=value.value
  value.value=''
  try {
    const {data}=await api.post(`/plugins/auth/${props.name}/complete`,{value:submitted})
    status.value={...data,message:'正在检查账号并保存连接，请稍候'}
  } catch(e) {error.value=e.response?.data?.error || '授权结果未能提交，请重新回填'}
  finally {submitting.value=false;emit('busy',working.value)}
}
async function poll() {
  if (!disposed && !document.hidden && !submitting.value) await refresh()
  if (!disposed) timer=setTimeout(poll,2000)
}
onMounted(() => {void poll()})
onUnmounted(() => {disposed=true;value.value='';clearTimeout(timer);emit('busy',false)})
</script>

<style scoped>
.auth-panel{background:var(--yt-surface);border:1px solid var(--yt-card-border);border-radius:16px;padding:24px;margin:18px 0}
h2{font-size:16px;margin:0 0 12px;color:var(--yt-ink-1)}
p{font-size:13px;line-height:1.8;color:var(--yt-ink-2)}
.auth-actions{display:flex;gap:18px;align-items:center;flex-wrap:wrap;margin:18px 0}
a{color:var(--yt-primary);font-size:13px}
label{display:block;font-size:13px;margin:20px 0 10px}
.n-input{max-width:750px;display:block;margin-bottom:14px}
.hint{font-size:12px;color:var(--yt-ink-3)}
.auth-status{margin-top:18px;overflow-wrap:anywhere}
@media(max-width:760px){.auth-panel{padding:18px}}
</style>
