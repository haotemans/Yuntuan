import { expect, test } from '@playwright/test'

function catalog() {
  return [
    ['nai-image', '琼华', 'NAI 生图与提示词优化', ['/image', '/画', '/prompt']],
    ['social-media', '采薇', '视频与图文解析下载', ['/下载', '/download']],
    ['anysearch', '采苓', '联网搜索与网页提取', ['/搜索', '/search']],
    ['moderator', '敬之', '撤回、禁言与群聊管理', ['/撤回', '/禁言']],
    ['tencent-docs', '腾讯文档', '官方 MCP 与在线文档 Skill', ['/文档']],
    ['quark-drive', '夸克网盘', '官方 CLI 与网盘 Skill', ['/夸克']],
    ['qzone', 'QQ空间', 'QQ空间说说发布与记录', ['/说说', '/空间记录']],
  ].map(([name, display_name, purpose, commands]) => ({
    name, display_name, description:purpose, version:'0.1.0', enabled:true, loaded:true, updating:false,
    pending_reload:false, pending_restart:false,
    presentation:{ purpose, examples: name==='nai-image' ? ['/image -m nai5 -r 2:3 成年女性在图书馆阅读'] : ['请直接描述任务'] },
    skills:[{ name:name==='qzone'?'qzone.publish':name+'.generate', description:purpose, commands }],
    tools:[{name:name==='qzone'?'qzone.publish':name+'.generate',description:purpose}],
    invocations:{active:0,succeeded:3,failed:0,rejected:0,cancelled:0,last_status:'succeeded',last_duration_ms:1250},
    config_fields: name==='nai-image' ? [
      {key:'api_key',label:'API Key',kind:'string',secret:true,has_value:true},
      {key:'base_url',label:'API 地址',kind:'string',default:'https://nai.rinko.ai'},
      {key:'default_model',label:'默认模型',kind:'choice',default:'nai4.5',options:[{value:'nai4.5',label:'NAI 4.5'},{value:'nai5',label:'NAI 5'}]},
      {key:'enabled_option',label:'启用提示词优化',kind:'boolean',default:true},
    ] : name==='qzone' ? [
      {key:'default_visibility',label:'默认可见范围',kind:'choice',default:'friends',options:[{value:'friends',label:'好友可见'},{value:'private',label:'仅自己可见'}]},
      {key:'daily_limit',label:'每日发布上限',kind:'integer',default:10},
    ] : [],
    settings:{enabled:true,allowed_chats:null,timeout_ms:360000,max_concurrency:1,max_output_bytes:65536,
      config:name==='nai-image'?{base_url:'https://nai.rinko.ai',default_model:'nai4.5',enabled_option:true}:name==='qzone'?{default_visibility:'friends',daily_limit:10}:{}},
  }))
}

async function openPlugins(page, { deferred=false,error=false }={}) {
  const state={plugins:catalog(),reads:0,writes:[],capabilityWrites:[],released:false}
  state.capabilities=state.plugins.flatMap(p=>[
    ...p.skills.map(s=>({name:s.name,kind:'skill',enabled:true,scopes:['group:42'],calls:3})),
    ...p.tools.map(t=>({name:t.name,kind:'tool',enabled:true,scopes:['group:42'],calls:2,tool_contract:{input_schema:{type:'object',properties:t.name==='qzone.publish'?{content:{type:'string'}}:{prompt:{type:'string'}}}}})),
  ])
  await page.addInitScript(() => localStorage.setItem('yt_token','test-only'))
  await page.routeWebSocket('**/ws**',()=>{})
  await page.route('**/api/**',async route=>{
    const pathname=new URL(route.request().url()).pathname
    if (pathname==='/api/capabilities') {
      if (route.request().method()==='POST') {
        const body=route.request().postDataJSON()
        state.capabilityWrites.push(body)
        Object.assign(state.capabilities.find(c=>c.name===body.name&&c.kind===body.kind),body)
        return route.fulfill({json:{ok:true}})
      }
      return route.fulfill({json:{capabilities:state.capabilities}})
    }
    if (pathname==='/api/plugins/list') {
      state.reads++
      if (state.released) state.plugins[0].updating=false
      return route.fulfill({json:{ok:true,plugins:state.plugins}})
    }
    if (['/api/plugins/config','/api/plugins/toggle'].includes(pathname)) {
      const body=route.request().postDataJSON()
      state.writes.push(body)
      if (error) return route.fulfill({status:400,json:{error:'API 地址必须是有效的 HTTP(S) 链接'}})
      const plugin=state.plugins.find(p=>p.name===body.name)
      if (body.settings) plugin.settings=body.settings
      else { plugin.enabled=body.enabled;plugin.loaded=body.enabled }
      plugin.updating=deferred && !state.released
      return route.fulfill({status:deferred?202:200,json:{ok:true,applying:deferred}})
    }
    return route.fulfill({json:{}})
  })
  await page.goto('/plugins')
  await expect(page.getByRole('heading',{name:'琼华',exact:true})).toBeVisible()
  return state
}

async function openConfig(page) {
  await page.locator('.plugin-card').filter({has:page.getByRole('heading',{name:'琼华',exact:true})})
    .getByRole('button',{name:'配置与用法'}).click()
  await expect(page).toHaveURL(/\/plugins\/nai-image\?tab=config/)
  await expect(page.getByRole('button',{name:'保存并应用'})).toBeVisible()
}

test('independent plugin pages, search, official instructions and responsive layout',async ({page})=>{
  await page.setViewportSize({width:1440,height:1000})
  await openPlugins(page)
  await expect(page.locator('.plugin-card h2')).toHaveText(['琼华','采薇','采苓','敬之','腾讯文档','夸克网盘','QQ空间'])
  await page.getByRole('button',{name:'查看琼华详情'}).click()
  await expect(page).toHaveURL(/\/plugins\/nai-image/)
  await expect(page.getByText('/image -m nai5 -r 2:3 成年女性在图书馆阅读',{exact:true})).toBeVisible()
  await page.getByRole('button',{name:'返回插件列表'}).click()
  await page.getByPlaceholder('搜索插件名称或用途').fill('搜索')
  await expect(page.locator('.plugin-card')).toHaveCount(1)
  await page.getByPlaceholder('搜索插件名称或用途').fill('')
  await page.setViewportSize({width:390,height:844})
  await expect(page.locator('.plugin-card')).toHaveCount(7)
  expect(await page.locator('.plugins-page').evaluate(el=>el.scrollWidth<=el.clientWidth)).toBe(true)
  await page.getByRole('button',{name:'查看腾讯文档详情'}).click()
  await expect(page.getByText('官方 MCP 与在线文档 Skill').first()).toBeVisible()
  await expect(page.getByRole('link',{name:/查看官方授权及安装说明/})).toHaveAttribute('href','https://docs.qq.com/open/auth/mcp.html')
})

test('deferred configuration save is sent only once and masks saved secrets',async ({page})=>{
  const state=await openPlugins(page,{deferred:true})
  await openConfig(page)
  await page.getByPlaceholder('已保存，留空保持不变').fill('')
  const address=page.locator('.n-form-item').filter({hasText:'API 地址'}).locator('input').first()
  await address.fill('https://example.com/v1')
  await page.getByRole('button',{name:'保存并应用'}).click()
  await expect(page.getByText('已提交配置，正在后台应用')).toBeVisible()
  expect(state.writes).toHaveLength(1)
  expect(state.writes[0].settings.config).not.toHaveProperty('api_key')
  await expect(page.getByRole('button',{name:'保存并应用'})).toBeDisabled()
  state.released=true
  await expect.poll(()=>state.reads,{timeout:8000}).toBeGreaterThan(2)
  expect(state.writes).toHaveLength(1)
})

test('unsaved drafts survive polling and failed saves display the error',async ({page})=>{
  const state=await openPlugins(page,{error:true})
  await openConfig(page)
  const secret=page.getByPlaceholder('已保存，留空保持不变')
  await secret.fill('not-a-real-key')
  const previous=state.reads
  await expect.poll(()=>state.reads,{timeout:8000}).toBeGreaterThan(previous)
  await expect(secret).toHaveValue('not-a-real-key')
  await page.getByRole('button',{name:'保存并应用'}).click()
  await expect(page.getByText('API 地址必须是有效的 HTTP(S) 链接')).toBeVisible()
  await expect(secret).toHaveValue('not-a-real-key')
  await expect(page.getByRole('button',{name:'保存并应用'})).toBeEnabled()
})

test('tool toggle retains scopes and is independent of the skill with identical name',async ({page})=>{
  const state=await openPlugins(page)
  await openConfig(page)
  await page.getByText('Skills / Tools',{exact:true}).click()
  const tool=page.getByRole('switch',{name:'工具 nai-image.generate'})
  await expect(tool).toBeVisible()
  const row=tool.locator('..');
  await row.getByText('调用参数',{exact:true}).click()
  await expect(row.locator('.tool-schema')).toContainText('prompt')
  await tool.click()
  await expect.poll(()=>state.capabilityWrites.length).toBe(1)
  expect(state.capabilityWrites[0]).toEqual({name:'nai-image.generate',kind:'tool',enabled:false,scopes:['group:42']})
  expect(state.capabilities.find(c=>c.name==='nai-image.generate'&&c.kind==='skill').enabled).toBe(true)
  await expect(tool).toHaveAttribute('aria-checked','false')
})


test('QQ Space uses the common independent settings route without asking for cookies',async ({page})=>{
  const state=await openPlugins(page)
  await page.getByRole('button',{name:'查看QQ空间详情'}).click()
  await expect(page).toHaveURL(/\/plugins\/qzone/)
  await expect(page.getByText('无需另填 Cookie',{exact:false})).toBeVisible()
  await expect(page.getByRole('switch',{name:'启用插件'})).toBeVisible()
  await page.getByText('连接配置',{exact:true}).click()
  const visibility=page.locator('.n-form-item').filter({hasText:'默认可见范围'})
  await visibility.locator('.n-select').click()
  await page.locator('.n-base-select-menu').getByText('仅自己可见',{exact:true}).click()
  await page.getByRole('button',{name:'保存并应用'}).click()
  await expect.poll(()=>state.writes.length).toBe(1)
  expect(state.writes[0].settings.config.default_visibility).toBe('private')
  await page.getByText('Skills / Tools',{exact:true}).click()
  const tool=page.getByRole('switch',{name:'工具 qzone.publish'})
  await expect(tool).toBeVisible()
  await tool.locator('..').getByText('调用参数',{exact:true}).click()
  await expect(tool.locator('..').locator('.tool-schema')).toContainText('content')
})
