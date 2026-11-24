<template>
  <div class="yt-page plugins-page">
    <header class="yt-page-head">
      <h1 class="yt-page-title">插件</h1>
      <div v-if="!loading && !loadError" class="yt-page-sub" aria-live="polite">{{ readyCount }} 个已加载 · {{ plugins.length }} 个内置插件</div>
    </header>
    <div class="yt-toolbar plugins-toolbar">
      <div class="plugin-filters" role="group" aria-label="筛选插件">
        <button v-for="item in filters" :key="item.value" :class="{ selected: filter === item.value }"
          :aria-pressed="filter === item.value" @click="filter = item.value">{{ item.label }} <span>{{ item.count }}</span></button>
      </div>
      <n-input v-model:value="query" class="plugin-search" clearable placeholder="搜索插件名称或用途" aria-label="搜索插件" />
    </div>
    <n-alert v-if="loadError" type="error" :bordered="false">{{ loadError }}
      <n-button size="small" text @click="load">重试</n-button>
    </n-alert>
    <n-alert v-if="actionError" type="error" :bordered="false" closable @close="actionError = ''">{{ actionError }}</n-alert>
    <div v-if="loading" class="plugins-grid"><n-skeleton v-for="i in 3" :key="i" height="300px" style="border-radius:16px" /></div>
    <div v-else-if="filtered.length" class="plugins-grid">
      <PluginCard v-for="plugin in filtered" :key="plugin.name" :plugin="plugin"
        :busy="!!pending[plugin.name]" @toggle="toggle" @open="openDetail" />
    </div>
    <div v-else-if="!loadError" class="plugins-empty">
      <EmptyState :title="plugins.length ? '未找到匹配插件' : '暂无已安装插件'" />
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useMessage } from 'naive-ui'
import { api } from '../api'
import PluginCard from '../components/PluginCard.vue'
import EmptyState from '../components/EmptyState.vue'
import { pluginStatus } from '../plugin-ui'

const router = useRouter()
const messages = useMessage()
const plugins = ref([])
const loading = ref(true)
const loadError = ref('')
const actionError = ref('')
const filter = ref('all')
const query = ref('')
const pending = ref({})
const readyCount = computed(() => plugins.value.filter(p => p.enabled && p.loaded).length)
const needsAttention = plugin => ['restart', 'attention', 'unavailable'].includes(pluginStatus(plugin).key)
const filters = computed(() => [
  { value:'all', label:'全部', count:plugins.value.length },
  { value:'enabled', label:'已启用', count:plugins.value.filter(p => p.enabled).length },
  { value:'attention', label:'待处理', count:plugins.value.filter(needsAttention).length },
])
const filtered = computed(() => plugins.value.filter(p => {
  if (filter.value === 'enabled' && !p.enabled) return false
  if (filter.value === 'attention' && !needsAttention(p)) return false
  return [p.name, p.display_name, p.description, p.presentation?.purpose].join(' ').toLowerCase().includes(query.value.toLowerCase().trim())
}))
let disposed = false
let timer
let inflight = false
async function load() {
  if (inflight || disposed) return
  inflight = true
  try {
    const {data} = await api.get('/plugins/list')
    if (disposed) return
    plugins.value = data.plugins || []
    loadError.value = ''
  } catch(e) {
    if (!disposed) loadError.value = e.response?.data?.error || '无法获取插件状态'
  } finally {
    loading.value = false
    inflight = false
  }
}
function openDetail(plugin, tab = 'overview') {
  router.push({ name:'plugin-settings', params:{name:plugin.name}, query:{tab} })
}
async function toggle(plugin, enabled) {
  if (pending.value[plugin.name]) return
  pending.value = {...pending.value, [plugin.name]:true}
  actionError.value = ''
  try {
    await api.post('/plugins/toggle', {name:plugin.name,enabled})
    messages.success('已提交插件状态更改')
    await load()
  } catch(e) {
    actionError.value = e.response?.data?.error || '更改插件状态失败'
  } finally {
    const next = {...pending.value}
    delete next[plugin.name]
    pending.value = next
  }
}
async function poll() {
  if (!disposed && !document.hidden) await load()
  if (!disposed) timer = setTimeout(poll, 2500)
}
onMounted(() => { void poll() })
onUnmounted(() => { disposed = true; clearTimeout(timer) })
</script>

<style scoped>
.plugins-page { max-width:1280px; padding-bottom:32px; }
.yt-page-title { margin: 0; }
.plugins-toolbar { justify-content:space-between; }
.plugin-filters { display:flex; flex-wrap:wrap; gap:8px; }
.plugin-filters button { color:var(--yt-ink-2); background:var(--yt-surface); border:1px solid var(--yt-card-border); padding:8px 13px; border-radius:9px; font:inherit; cursor:pointer; font-size:12px; }
.plugin-filters button.selected { color:var(--yt-primary); border-color:var(--yt-primary); }
.plugin-filters span { opacity:.6; margin-left:6px; }
.plugin-search { max-width:260px; }
.plugins-grid { display:grid; grid-template-columns:repeat(3,minmax(0,1fr)); gap:18px; }
.plugins-empty { padding:40px 0; }
@media(max-width:1100px) { .plugins-grid { grid-template-columns:repeat(2,minmax(0,1fr)); } }
@media(max-width:720px) {
  .plugins-toolbar {align-items:stretch;}
  .plugin-search {max-width:none;}
  .plugins-grid {grid-template-columns:1fr;}
}
</style>
