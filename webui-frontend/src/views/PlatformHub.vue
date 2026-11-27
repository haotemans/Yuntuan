<template>
  <div class="yt-page platform-hub">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">{{ isMemories ? '记忆库' : '关系图谱' }}</span>
        <span class="yt-page-sub">{{ isMemories ? '按平台隔离浏览人物画像、记忆与原始发言' : '按平台独立查看人物关系图谱' }}</span>
      </div>
      <n-button size="small" secondary :loading="loading" @click="load">刷新</n-button>
    </div>

    <n-alert v-if="statsError" type="error" role="alert" :bordered="false" class="hub-alert">
      {{ statsError }}
      <n-button text size="tiny" @click="load">重试</n-button>
    </n-alert>

    <div class="platform-grid">
      <article v-for="platform in PLATFORMS" :key="platform.id" class="channel-card" :data-platform="platform.id">
        <button
          type="button"
          class="channel-open"
          :aria-label="`进入 ${platform.title}${isMemories ? '记忆库' : '关系图谱'}`"
          @click="openPlatform(platform.id)"
        >
          <div class="channel-head">
            <h2 class="channel-title">{{ platform.title }}</h2>
            <n-tag size="small" :type="statusTone(statusOf(platform.id)?.state)">
              {{ STATUS_LABELS[statusOf(platform.id)?.state] || '状态未知' }}
            </n-tag>
          </div>
          <p class="channel-description">{{ isMemories ? platform.memoriesDescription : platform.relationsDescription }}</p>
          <dl class="channel-facts">
            <div>
              <dt>运行账号</dt>
              <dd class="mono">{{ statusOf(platform.id)?.account || '—' }}</dd>
            </div>
            <template v-if="isMemories">
              <div>
                <dt>人物画像</dt>
                <dd class="mono">{{ countOf(platform.id, 'people') }}</dd>
              </div>
              <div>
                <dt>历史记忆</dt>
                <dd class="mono">{{ countOf(platform.id, 'memories') }}</dd>
              </div>
              <div>
                <dt>每日摘要</dt>
                <dd class="mono">{{ countOf(platform.id, 'summaries') }}</dd>
              </div>
            </template>
            <template v-else>
              <div>
                <dt>人物节点</dt>
                <dd class="mono">{{ countOf(platform.id, 'nodes') }}</dd>
              </div>
              <div>
                <dt>关系连线</dt>
                <dd class="mono">{{ countOf(platform.id, 'edges') }}</dd>
              </div>
            </template>
          </dl>
          <div class="channel-footer">
            <span>{{ isMemories ? '进入记忆库' : '进入关系图谱' }}</span>
            <span aria-hidden="true">›</span>
          </div>
        </button>
      </article>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../api'
import { PLATFORMS, STATUS_LABELS } from '../platforms'

const props = defineProps({
  section: {
    type: String,
    required: true,
  },
})

const route = useRoute()
const router = useRouter()
const isMemories = computed(() => props.section === 'memories')
const statuses = ref([])
const platformStats = ref([])
const loading = ref(false)
const loadedOnce = ref(false)
const statsError = ref('')
let controller
let disposed = false

function statusOf(id) {
  return statuses.value.find(item => item.id === id)
}

function statsOf(id) {
  return platformStats.value.find(item => item.id === id)
}

function countOf(id, key) {
  const entry = statsOf(id)
  if (!entry || entry[key] === undefined || entry[key] === null) {
    return loading.value && !loadedOnce.value ? '...' : '—'
  }
  return entry[key]
}

function statusTone(state) {
  if (state === 'online') return 'success'
  if (state === 'error') return 'error'
  return 'default'
}

function openPlatform(id) {
  router.push({
    name: isMemories.value ? 'memories-platform' : 'relations-platform',
    params: { platform: id },
  })
}

function redirectDeepLink() {
  if (isMemories.value || typeof route.query.person !== 'string' || !route.query.person) return false
  const target = route.query.person.startsWith('qq-official_') ? 'qq_official' : 'onebot'
  void router.replace({
    name: 'relations-platform',
    params: { platform: target },
    query: route.query,
  })
  return true
}

async function load() {
  controller?.abort()
  const request = controller = new AbortController()
  loading.value = true
  statsError.value = ''
  try {
    const [statusResult, statsResult] = await Promise.allSettled([
      api.get('/platforms', { signal: request.signal }),
      api.get('/memories/platforms', { signal: request.signal }),
    ])
    if (request.signal.aborted || disposed) return
    if (statusResult.status === 'fulfilled') {
      statuses.value = statusResult.value.data?.platforms || []
    }
    if (statsResult.status === 'fulfilled') {
      platformStats.value = statsResult.value.data?.platforms || []
    } else {
      statsError.value = statsResult.reason?.response?.data?.error || '无法读取平台统计数据'
    }
    loadedOnce.value = true
  } finally {
    if (controller === request) loading.value = false
  }
}

watch(() => [props.section, route.query.person], () => {
  redirectDeepLink()
})

onMounted(() => {
  if (redirectDeepLink()) return
  void load()
})

onUnmounted(() => {
  disposed = true
  controller?.abort()
})
</script>

<style scoped>
.hub-alert { margin-bottom: 16px; }
.platform-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 340px), 1fr)); gap: 16px; align-items: start; }
.channel-card { display: flex; flex-direction: column; background: var(--yt-surface); border: 1px solid var(--yt-card-border); border-radius: 12px; box-shadow: var(--yt-card-shadow); overflow: hidden; transition: border-color 0.15s ease; }
.channel-card:hover { border-color: var(--yt-primary); }
.channel-open { display: flex; flex-direction: column; width: 100%; padding: 18px; border: 0; text-align: left; color: inherit; background: transparent; cursor: pointer; font: inherit; }
.channel-open:focus-visible { outline: 2px solid var(--yt-primary); outline-offset: -4px; border-radius: 12px; }
.channel-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 10px; }
.channel-title { margin: 0; font-size: 17px; font-weight: 600; color: var(--yt-ink-1); }
.channel-open:hover .channel-title { color: var(--yt-primary); }
.channel-description { margin: 0; min-height: 40px; color: var(--yt-ink-2); font-size: 13px; line-height: 1.55; }
.channel-facts { display: grid; grid-template-columns: repeat(auto-fit, minmax(110px, 1fr)); gap: 12px; margin: 18px 0 14px; }
dt { color: var(--yt-ink-3); font-size: 12px; }
dd { margin: 4px 0 0; overflow-wrap: anywhere; font-weight: 600; color: var(--yt-ink-1); }
.channel-footer { display: flex; align-items: center; justify-content: space-between; padding-top: 12px; border-top: 1px solid var(--yt-card-border); font-size: 13px; color: var(--yt-primary); font-weight: 500; }
.channel-footer span[aria-hidden='true'] { font-size: 19px; line-height: 14px; }
</style>
