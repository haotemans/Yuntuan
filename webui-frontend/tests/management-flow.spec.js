import {expect,test} from '@playwright/test'
async function setup(page){
 await page.addInitScript(()=>localStorage.setItem('yt_token','test-only'))
 await page.routeWebSocket('**/ws**',()=>{})
 const state={policy:{owner_qq:'123',configured_owner_qq:'123',grants:[],restart_supported:true},plugins:[{name:'anysearch',display_name:'采苓',version:'1',revision:'v1',enabled:true,loaded:true}],history:[]}
 const writes=[]
 await page.route('**/api/**',route=>{
  const req=route.request(),path=new URL(req.url()).pathname
  if(path==='/api/management')return route.fulfill({json:state})
  if(req.method()==='GET')return route.fulfill({json:{}})
  const body=req.postDataJSON();writes.push({path,body})
  if(path==='/api/management/permissions')state.policy.grants=body.grants
  if(path==='/api/management/owner'){state.policy.configured_owner_qq=body.qq;return route.fulfill({json:{requires_restart:['security.super_admin_qq']}})}
  if(path==='/api/management/plugins'){
   state.plugins[0].revision='v2'
   state.history.unshift({id:1,ts:1800000000,record:{actor:'webui',action:body.operation,plugin:body.plugin,status:'applied'}})
   return route.fulfill({json:{status:'applied',message:'操作已生效'}})
  }
  if(path==='/api/config/restart')return route.fulfill({json:{ok:true,msg:'云团正在收尾并重启，请稍后刷新'}})
  return route.fulfill({json:{ok:true}})
 })
 return{state,writes}
}
test('owner grants are explicit per plugin, drafts survive refresh and changes carry revision',async({page})=>{
 const {writes}=await setup(page)
 await page.setViewportSize({width:1440,height:1000})
 await page.goto('/management')
 await expect(page.getByText('当前主人 123',{exact:true})).toBeVisible()
 await page.getByRole('button',{name:'添加管理员'}).click()
 await page.getByLabel('管理员 QQ 1').fill('456')
 await page.getByLabel('管理员插件 1').click();await page.getByText('采苓',{exact:true}).last().click()
 await page.getByText('修改常规配置',{exact:true}).click()
 await page.getByRole('button',{name:'刷新',exact:true}).click()
 await expect(page.getByLabel('管理员 QQ 1')).toHaveValue('456')
 await page.getByRole('button',{name:'保存授权'}).click()
 expect(writes[0]).toEqual({path:'/api/management/permissions',body:{grants:[{qq:'456',plugins:['anysearch'],configure:true,toggle:false,reload:false}]}})
 await page.getByRole('button',{name:'重载',exact:true}).click()
 await expect(page.getByText('操作已生效',{exact:true})).toBeVisible()
 expect(writes[1].body).toEqual({plugin:'anysearch',operation:'reload',revision:'v1'})
 await expect(page.locator('.history-list')).toContainText('已生效')
 await page.getByLabel('主人 QQ ID').fill('789')
 await page.getByRole('button',{name:'保存主人 ID'}).click()
 await expect(page.getByText('新主人 789 已保存，重启云团后生效。')).toBeVisible()
 await expect(page.getByText('当前主人 123',{exact:true})).toBeVisible()
 await page.screenshot({path:'test-results/management-desktop.png',fullPage:true})
 await page.setViewportSize({width:390,height:844})
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true)
 await page.getByRole('button',{name:'切换主题'}).click()
 await expect(page.locator('html')).toHaveClass('dark')
 await page.screenshot({path:'test-results/management-mobile-dark.png',fullPage:true})
})
test('failed requests retain grants and restart requires a deliberate click',async({page})=>{
 const {writes}=await setup(page)
 await page.route('**/api/management/permissions',route=>route.fulfill({status:400,json:{error:'管理员 QQ 必须是完整数字 ID'}}))
 await page.goto('/management')
 await page.getByRole('button',{name:'添加管理员'}).click()
 await page.getByLabel('管理员 QQ 1').fill('bad-id')
 await page.getByRole('button',{name:'保存授权'}).click()
 await expect(page.getByText('管理员 QQ 必须是完整数字 ID')).toBeVisible()
 await expect(page.getByLabel('管理员 QQ 1')).toHaveValue('bad-id')
 await page.locator('#app').getByRole('button',{name:'重启云团',exact:true}).click()
 expect(writes).toEqual([])
 await page.getByRole('button',{name:'取消',exact:true}).click()
 expect(writes).toEqual([])
 await page.locator('#app').getByRole('button',{name:'重启云团',exact:true}).click()
 await page.locator('.n-popover').getByRole('button',{name:'重启云团',exact:true}).click()
 await expect(page.getByText('云团正在收尾并重启，请稍后刷新')).toBeVisible()
 expect(writes).toHaveLength(1)
 expect(writes[0].path).toBe('/api/config/restart')
})
