import { createRouter, createWebHistory } from 'vue-router'
import { isLoggedIn } from '../api'

/** @type {import('vue-router').RouteRecordRaw[]} */
const routes = [
  { path: '/login', name: 'login', component: () => import('../views/Login.vue') },
  { path: '/', name: 'dashboard', component: () => import('../views/Dashboard.vue') },
  { path: '/trace', name: 'trace', component: () => import('../views/Trace.vue') },
  { path: '/logs', name: 'logs', component: () => import('../views/Logs.vue') },
  { path: '/tasks', name: 'tasks', component: () => import('../views/Tasks.vue') },
  { path: '/schedules', name: 'schedules', component: () => import('../views/Schedules.vue') },
  { path: '/memories', name: 'memories', meta: { menu: 'memories' }, component: () => import('../views/PlatformHub.vue'), props: { section: 'memories' } },
  { path: '/memories/:platform', name: 'memories-platform', meta: { menu: 'memories' }, component: () => import('../views/Memories.vue') },
  { path: '/relations', name: 'relations', meta: { menu: 'relations' }, component: () => import('../views/PlatformHub.vue'), props: { section: 'relations' } },
  { path: '/relations/:platform', name: 'relations-platform', meta: { menu: 'relations' }, component: () => import('../views/Relations.vue') },
  { path: '/platform', name: 'platform', component: () => import('../views/Platform.vue') },
  { path: '/models', name: 'models', component: () => import('../views/Models.vue') },
  { path: '/params', name: 'params', component: () => import('../views/Params.vue') },
  { path: '/personality', name: 'personality', component: () => import('../views/Personality.vue') },
  { path: '/meme', name: 'meme', component: () => import('../views/Meme.vue') },
  { path: '/kb', name: 'kb', component: () => import('../views/Kb.vue') },
  { path: '/plugins', name: 'plugins', component: () => import('../views/Plugins.vue') },
  { path: '/management', name: 'management', component: () => import('../views/Management.vue') },
  { path: '/plugins/:name', name: 'plugin-settings', meta: { menu: 'plugins' }, component: () => import('../views/PluginSettings.vue') },
  { path: '/mcp', name: 'mcp', component: () => import('../views/Mcp.vue') },
  { path: '/backup', name: 'backup', component: () => import('../views/Backup.vue') },
  { path: '/:pathMatch(.*)*', redirect: '/' },
]

const router = createRouter({ history: createWebHistory(), routes })

router.beforeEach((to) => {
  if (to.name !== 'login' && !isLoggedIn()) return { name: 'login' }
  if (to.name === 'login' && isLoggedIn()) return { name: 'dashboard' }
})

export default router
