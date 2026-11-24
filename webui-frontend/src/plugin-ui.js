export function pluginStatus(plugin, pending = false) {
  if (pending || plugin.updating) return { key: 'applying', label: '正在应用', tone: 'working' }
  if (plugin.pending_restart) return { key: 'restart', label: '需要重启服务', tone: 'error' }
  if (plugin.pending_reload || plugin.error) return { key: 'attention', label: '需要处理', tone: 'warning' }
  if (!plugin.enabled) return { key: 'disabled', label: '未启用', tone: 'muted' }
  if (!plugin.loaded) return { key: 'unavailable', label: '未加载', tone: 'warning' }
  if (plugin.invocations?.active) return { key: 'running', label: `${plugin.invocations.active} 个任务执行中`, tone: 'working' }
  return { key: 'ready', label: '已启用', tone: 'success' }
}

export function pluginDuration(milliseconds) {
  if (milliseconds == null) return '—'
  if (milliseconds < 1000) return `${milliseconds} ms`
  return `${(milliseconds / 1000).toFixed(1)} s`
}

export function scopePreview(plugin, capability, chatType, chatId, onebotState) {
  if (plugin.updating || plugin.pending_reload || plugin.pending_restart) return '等待插件配置生效'
  if (!plugin.enabled) return '插件未启用'
  if (!plugin.loaded || plugin.error) return '插件未加载'
  if (plugin.requires_group && chatType !== 'group') return '该插件只支持群聊'
  if (plugin.requires_onebot && chatId.startsWith('qq-official_')) return '该插件需要 OneBot'
  if (plugin.settings?.allowed_chats && !plugin.settings.allowed_chats.includes(chatId)) return '插件未授权此会话'
  if (!capability) return '能力配置不可用'
  if (!capability.enabled) return '能力已停用'
  if (capability.scopes?.length && !capability.scopes.includes(`${chatType}:${chatId}`)) return '能力未授权此会话'
  if (plugin.requires_onebot && onebotState !== 'online') {
    return onebotState ? 'OneBot 未连接' : 'OneBot 连接状态未知'
  }
  return ''
}
