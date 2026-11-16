const fs=require('node:fs')
const path=require('node:path')
async function check() {
  const shared=process.env.YUANTUAN_QUARK_SHARED_DIR || '/app/data/connectors'
  const password=fs.readFileSync(path.join(shared,'quark-service/password'),'utf8').trim()
  const response=await fetch('http://127.0.0.1:4096/global/health',{
    signal:AbortSignal.timeout(2500),headers:{authorization:'Basic '+Buffer.from('opencode:'+password).toString('base64')}
  })
  if(!response.ok || !(await response.json()).healthy) throw new Error('unhealthy')
}
check().then(()=>process.exit(0),()=>process.exit(1))
