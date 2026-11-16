const fs=require('node:fs')
const {spawn}=require('node:child_process')
const path=require('node:path')
const shared=process.env.YUANTUAN_QUARK_SHARED_DIR || '/app/data/connectors'
const password=fs.readFileSync(path.join(shared,'quark-service/password'),'utf8').trim()
if(password.length<32 || password.length>256 || /[\r\n\0]/.test(password)) throw new Error('Quark worker credential is not configured')
const child=spawn('opencode',['serve','--hostname','0.0.0.0','--port','4096'],{
  stdio:'inherit',env:{...process.env,OPENCODE_SERVER_PASSWORD:password,
    OPENCODE_CONFIG_CONTENT:JSON.stringify({autoupdate:false,plugin:[],permission:{'*':'deny',bash:'allow'}})}
})
for(const signal of ['SIGTERM','SIGINT'])process.on(signal,()=>child.kill(signal))
child.on('exit',(code)=>process.exit(code ?? 1))
