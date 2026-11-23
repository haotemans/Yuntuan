<template>
  <n-card size="small" title="会话授权预检">
    <form class="yt-toolbar preflight-form" @submit.prevent="check">
      <n-select v-model:value="chatType" :options="chatTypes" size="small" aria-label="预检会话类型" />
      <n-input v-model:value="chatId" size="small" placeholder="填写会话 ID" :input-props="{ 'aria-label': '预检会话 ID' }" />
      <n-button attr-type="submit" size="small" secondary :loading="loading" :disabled="!validId">检查会话</n-button>
    </form>
    <div v-if="checked" class="preflight-result" aria-live="polite">
      <p>预检读取插件状态和会话授权。调用时还会核验发起人、平台权限和参数。</p>
      <p v-if="!rows.length">当前没有已注册的能力。</p>
      <ul v-else>
        <li v-for="row in rows" :key="`${row.kind}:${row.name}`">
          <code>{{ row.name }} <small>{{ row.kind === 'skill' ? 'Skill' : 'Tool' }}</small></code>
          <n-tag size="small" :type="row.reason ? 'warning' : 'success'" :bordered="false">{{ row.reason || '会话配置允许' }}</n-tag>
        </li>
      </ul>
    </div>
  </n-card>
</template>

<script setup>
import { computed, onUnmounted, ref, watch } from 'vue'
import { api } from '../api'
import { scopePreview } from '../plugin-ui'

/** @typedef {{name:string, kind:string, enabled:boolean, scopes?:string[]}} Capability */
const props = defineProps({
  plugin: { type: Object, required: true },
  capabilities: { type: /** @type {import('vue').PropType<Capability[]>} */ (Array), required: true },
})
const chatTypes = [{ label:'群聊', value:'group' }, { label:'私聊', value:'private' }]
const chatType = ref('group')
const chatId = ref('')
const checked = ref(false)
const loading = ref(false)
const onebot = ref(null)
let request
const validId = computed(() => /^\S{1,192}$/.test(chatId.value.trim()))
const rows = computed(() => [['skill', props.plugin.skills], ['tool', props.plugin.tools]].flatMap(([kind, entries]) =>
  (entries || []).map(({ name }) => {
    const capability = props.capabilities.find(item => item.kind === kind && item.name === name)
    return { name, kind, reason: scopePreview(props.plugin, capability, chatType.value, chatId.value.trim(), onebot.value) }
  }),
))

function clearPreview() {
  request?.abort()
  checked.value = false
  loading.value = false
  onebot.value = null
}
async function check() {
  if (!validId.value) return
  clearPreview()
  const active = new AbortController()
  request = active
  if (props.plugin.requires_onebot && !chatId.value.trim().startsWith('qq-official_')) {
    loading.value = true
    try {
      const { data } = await api.get('/platforms', { signal: active.signal })
      if (!active.signal.aborted) onebot.value = data.platforms?.find(item => item.id === 'onebot')?.state || null
    } catch {
      if (!active.signal.aborted) onebot.value = null
    } finally {
      if (!active.signal.aborted) loading.value = false
    }
  }
  if (!active.signal.aborted) checked.value = true
}
watch([chatId, chatType, () => props.plugin.name], clearPreview)
onUnmounted(clearPreview)
</script>

<style scoped>
.preflight-form { padding: 0; border: 0; box-shadow: none; }
.preflight-form :deep(.n-select) { width: 100px; }
.preflight-form :deep(.n-input) { flex: 1 1 160px; }
.preflight-result p { color: var(--yt-ink-3); font-size: 13px; }
.preflight-result ul { list-style: none; margin: 0; padding: 0; }
.preflight-result li { display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 8px; padding: 8px 0; border-top: 1px solid var(--yt-card-border); overflow-wrap: anywhere; }
.preflight-result small { color: var(--yt-ink-3); }
</style>
