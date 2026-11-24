<template>
  <main class="yt-page management-page">
    <header class="yt-page-head"><div><h1 class="yt-page-title">机器人管理</h1><p class="yt-page-sub">身份授权、插件维护与应用重启</p></div><n-button size="small" secondary :loading="loading" @click="load">刷新</n-button></header>
    <n-alert v-if="error" type="error" :bordered="false">{{ error }}</n-alert>
    <n-alert v-if="notice" type="info" :bordered="false">{{ notice }}</n-alert>
    <div class="management-grid">
      <n-card title="主人身份" size="small">
        <p class="management-note">主人拥有聊天管理和应用重启权限。群主、群管理员和记忆库身份关联不授予机器人管理权限；聊天管理仅在 QQ 私聊开放。</p>
        <n-tag :type="policy.owner_qq ? 'success' : 'warning'" size="small" :bordered="false">{{ policy.owner_qq ? `当前主人 ${policy.owner_qq}` : '尚未设置主人，聊天管理已关闭' }}</n-tag>
        <n-alert v-if="policy.configured_owner_qq && policy.configured_owner_qq !== policy.owner_qq" type="warning" :bordered="false" class="block-gap">新主人 {{ policy.configured_owner_qq }} 已保存，重启云团后生效。</n-alert>
        <n-form label-placement="top" :show-feedback="false" class="block-gap" @submit.prevent="saveOwner"><n-form-item label="主人 QQ ID"><n-input v-model:value="owner" :input-props="{'aria-label':'主人 QQ ID',inputmode:'numeric'}" placeholder="填写完整 QQ 号" /></n-form-item><n-button attr-type="submit" size="small" secondary :loading="savingOwner" :disabled="!/^\d+$/.test(owner)">保存主人 ID</n-button></n-form>
        <p class="management-note">网页管理员会话拥有主人级管理权限，请仅由可信人员使用。</p>
      </n-card>
      <n-card title="运行维护" size="small">
        <div class="operation-line"><div><strong>配置与插件重载</strong><p>大部分插件配置保存后即可热重载；失败时可查看插件状态和运行日志。</p></div><n-tag size="small" type="success" :bordered="false">无需重启应用</n-tag></div>
        <div class="operation-line"><div><strong>重启云团</strong><p>停止接收消息、收尾发送队列并关闭数据库后，由进程守护拉起。QQ 登录容器保持运行。</p></div><n-tag size="small" :type="policy.restart_supported ? 'success' : 'warning'" :bordered="false">{{ policy.restart_supported ? '已配置守护' : '守护未配置' }}</n-tag></div>
        <div class="management-actions"><router-link to="/logs">查看运行日志</router-link><n-popconfirm positive-text="重启云团" negative-text="取消" @positive-click="restart"><template #trigger><n-button size="small" type="warning" secondary :loading="restarting" :disabled="!policy.restart_supported || restarting">重启云团</n-button></template>重启会短暂中断机器人服务，未完成的外部操作需核实；完成后请重新登录管理页。</n-popconfirm></div>
        <p class="management-note">当前插件随应用编译分发，新插件安装或代码更新需发布应用版本。此页支持现有插件的配置、启停与重载。</p>
      </n-card>
    </div>
    <n-card title="运维管理员" size="small">
      <template #header-extra><n-button size="small" secondary @click="addGrant">添加管理员</n-button></template>
      <p class="management-note">只授予选定插件的权限；运维管理员不能修改主人或授权、查看密钥、安装代码或重启应用。授权保存立即生效。</p>
      <div v-if="!grants.length" class="empty-note">尚未授权其他管理员，只有主人能够通过私聊管理。</div>
      <article v-for="(grant,index) in grants" :key="index" class="grant-row">
        <n-form-item label="QQ ID" :show-feedback="false"><n-input v-model:value="grant.qq" :input-props="{'aria-label':`管理员 QQ ${index+1}`,inputmode:'numeric'}" /></n-form-item>
        <n-form-item label="允许管理的插件" :show-feedback="false"><n-select v-model:value="grant.plugins" multiple :options="pluginOptions" :aria-label="`管理员插件 ${index+1}`" /></n-form-item>
        <div class="grant-switches"><n-checkbox v-model:checked="grant.configure">修改常规配置</n-checkbox><n-checkbox v-model:checked="grant.toggle">启停插件</n-checkbox><n-checkbox v-model:checked="grant.reload">重载插件</n-checkbox></div>
        <n-button size="small" text type="error" :aria-label="`移除管理员 ${index+1}`" @click="grants.splice(index,1); dirty=true">移除</n-button>
      </article>
      <div class="management-actions"><n-button type="primary" size="small" :loading="savingGrants" @click="saveGrants">保存授权</n-button><span v-if="dirty" class="management-note">授权草稿尚未保存</span></div>
    </n-card>
    <n-card title="插件维护" size="small">
      <p class="management-note">私聊示例：“检查采苓状态，把默认结果数改成 5 并保存”。云团先读取当前版本，再修改授权范围内的参数，并报告实际应用状态。凭据、地址和路径通过插件配置页管理。</p>
      <article v-for="plugin in plugins" :key="plugin.name" class="plugin-operation">
        <div><strong>{{ plugin.display_name || plugin.name }}</strong><p>{{ plugin.name }} · {{ plugin.version }}</p><n-tag size="small" :type="plugin.pending_restart || plugin.has_error ? 'warning' : plugin.loaded ? 'success' : 'default'" :bordered="false">{{ plugin.pending_restart ? '需要重启' : plugin.updating ? '正在应用' : plugin.pending_reload ? '待重新应用' : plugin.loaded ? '已加载' : '未加载' }}</n-tag></div>
        <div class="management-actions"><router-link :to="`/plugins/${plugin.name}`">配置与详情</router-link><n-button size="small" secondary :disabled="busy.has(plugin.name) || plugin.updating || plugin.pending_restart" @click="apply(plugin, plugin.enabled ? 'disable' : 'enable')">{{ plugin.enabled ? '停用' : '启用' }}</n-button><n-button size="small" secondary :loading="busy.has(plugin.name)" :disabled="busy.has(plugin.name) || plugin.updating || plugin.pending_restart" @click="apply(plugin,'reload')">重载</n-button></div>
      </article>
    </n-card>
    <n-card title="管理记录" size="small"><div class="history-list"><article v-for="item in history" :key="item.id"><time>{{ time(item.ts) }}</time><div><strong>{{ actionLabel(item.record.action) }}<template v-if="item.record.plugin"> · {{ item.record.plugin }}</template></strong><p>{{ item.record.actor }}<template v-if="item.record.fields?.length"> · 字段：{{ item.record.fields.join('、') }}</template></p></div><n-tag size="small" :bordered="false" :type="item.record.status === 'applied' ? 'success' : item.record.status === 'denied' || item.record.status === 'failed' ? 'warning' : 'default'">{{ statusLabel(item.record.status) }}</n-tag></article><p v-if="!history.length" class="management-note">暂无管理操作记录</p></div></n-card>
  </main>
</template>

<script setup>
import { onMounted,onUnmounted,reactive,ref,computed,watch } from 'vue'
import { api } from '../api'
const policy=ref({owner_qq:null,configured_owner_qq:null,grants:[],restart_supported:false}),plugins=ref([]),history=ref([]),grants=ref([]),owner=ref('')
const error=ref(''),notice=ref(''),loading=ref(false),savingOwner=ref(false),savingGrants=ref(false),restarting=ref(false),dirty=ref(false)
const busy=reactive(new Set())
const pluginOptions=computed(()=>plugins.value.map(p=>({label:p.display_name || p.name,value:p.name})))
const time=ts=>new Date(ts*1000).toLocaleString('zh-CN',{hour12:false})
const actionLabel=v=>({permissions:'修改授权',configure:'修改配置',enable:'启用',disable:'停用',reload:'重载',restart:'重启云团'})[v] || v
const statusLabel=v=>({accepted:'已受理',applying:'正在应用',applied:'已生效',denied:'权限拒绝',failed:'未生效',attention:'需检查',superseded:'已有后续修改'})[v] || v
let stopped=false,timer,controller,initialized=false
watch(grants,()=>{if(initialized)dirty.value=true},{deep:true,flush:'sync'})
async function load(initial=false){
  if(loading.value || stopped || restarting.value)return
  loading.value=true;controller=new AbortController()
  try{
    const {data}=await api.get('/management',{signal:controller.signal});if(stopped)return
    policy.value=data.policy || {};plugins.value=data.plugins || [];history.value=data.history || []
    if(!initialized || initial===true){grants.value=JSON.parse(JSON.stringify(policy.value.grants || []));owner.value=policy.value.configured_owner_qq || policy.value.owner_qq || '';dirty.value=false;initialized=true}
  }catch(e){if(!stopped)error.value=e.response?.data?.error || '无法读取机器人管理状态'}finally{loading.value=false}
}
function addGrant(){grants.value.push({qq:'',plugins:[],configure:false,toggle:false,reload:false});dirty.value=true}
async function saveGrants(){savingGrants.value=true;error.value='';try{await api.post('/management/permissions',{grants:grants.value});notice.value='授权已保存，新的聊天管理操作立即使用当前权限';dirty.value=false;await load()}catch(e){error.value=e.response?.data?.error || '保存授权失败'}finally{savingGrants.value=false}}
async function saveOwner(){savingOwner.value=true;error.value='';try{const {data}=await api.post('/management/owner',{qq:owner.value});notice.value=(data.requires_restart || []).length ? '主人 ID 已保存，重启云团后生效' : '主人 ID 已保存';await load()}catch(e){error.value=e.response?.data?.error || '保存主人失败'}finally{savingOwner.value=false}}
async function apply(plugin,operation){if(busy.has(plugin.name))return;busy.add(plugin.name);error.value='';try{const {data}=await api.post('/management/plugins',{plugin:plugin.name,operation,revision:plugin.revision});notice.value=data.message;await load()}catch(e){error.value=e.response?.data?.error || '插件操作失败';await load()}finally{busy.delete(plugin.name)}}
async function restart(){restarting.value=true;error.value='';try{const {data}=await api.post('/config/restart');notice.value=data.msg || '重启请求已受理，完成后请重新登录';localStorage.removeItem('yt_token')}catch(e){error.value=e.response?.data?.error || '重启请求失败';restarting.value=false}}
onMounted(()=>{void load(true);timer=setInterval(()=>{if(!document.hidden)void load()},5000)})
onUnmounted(()=>{stopped=true;clearInterval(timer);controller?.abort()})
</script>

<style scoped>
.management-page{display:grid;gap:16px;max-width:1320px}.management-grid{display:grid;grid-template-columns:1fr 1fr;gap:16px}.management-grid>*{min-width:0}.yt-page-title{margin:0}.yt-page-sub{margin:6px 0 0}.management-note,.operation-line p{font-size:12px;color:var(--yt-ink-3);line-height:1.8}.management-note{margin:8px 0 14px}.block-gap{margin-top:14px}.block-gap :deep(.n-form-item){margin-bottom:12px}.operation-line{display:flex;justify-content:space-between;align-items:flex-start;gap:16px;padding:10px 0}.operation-line+.operation-line{border-top:1px solid var(--yt-card-border)}.management-actions{display:flex;align-items:center;gap:12px;flex-wrap:wrap}.management-actions a{font-size:13px;color:var(--yt-primary);text-decoration:none}.grant-row{display:grid;grid-template-columns:180px minmax(200px,1fr) auto auto;align-items:end;gap:14px;padding:16px 0;border-top:1px solid var(--yt-card-border)}.grant-switches{display:grid;gap:8px;font-size:12px}.empty-note{color:var(--yt-ink-3);font-size:13px;padding:16px 0 24px}.plugin-operation{display:flex;justify-content:space-between;align-items:center;gap:16px;padding:16px 0;border-bottom:1px solid var(--yt-card-border)}.plugin-operation:last-child{border:0}.plugin-operation p{color:var(--yt-ink-3);font-size:12px;margin:5px 0 8px}.history-list article{display:grid;grid-template-columns:180px 1fr auto;align-items:center;gap:14px;padding:12px 0;border-bottom:1px solid var(--yt-card-border);font-size:13px}.history-list time,.history-list p{color:var(--yt-ink-3);font-size:12px}.history-list p{margin:5px 0;overflow-wrap:anywhere}.history-list strong{overflow-wrap:anywhere}
@media(max-width:1000px){.grant-row{grid-template-columns:1fr 1fr}.grant-switches{display:flex;flex-wrap:wrap}.management-grid{grid-template-columns:1fr}}@media(max-width:600px){.grant-row{grid-template-columns:1fr}.plugin-operation{align-items:flex-start;flex-direction:column}.history-list article{grid-template-columns:1fr auto}.history-list time{grid-column:1/-1}.operation-line{flex-wrap:wrap}}
</style>
