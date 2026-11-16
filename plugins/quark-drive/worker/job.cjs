// Executed by the real OpenCode shell API. Never forge Agent identity markers.
const fs = require('node:fs')
const path = require('node:path')
const {spawn} = require('node:child_process')
const operations = new Set(['resolve-agent','login','get-user-info','search','share-detail','get-share-update-list','get-share-update-files','share'])
const flags = new Set(['--token','--keyword','--url','--expired-type','--url-type','--session-input','--session-id'])
async function runJob(id, shared = process.env.YUANTUAN_QUARK_SHARED_DIR || '/app/data/connectors') {
  if (!/^[a-zA-Z0-9]{32}$/.test(id)) throw new Error('invalid ticket')
  const jobs = path.join(shared,'quark-service','jobs')
  const input = path.join(jobs,`${id}.input.json`)
  const output = path.join(jobs,`${id}.output.json`)
  const active = path.join(jobs,`${id}.active`)
  const fd = fs.openSync(input,fs.constants.O_RDONLY | (fs.constants.O_NOFOLLOW || 0))
  let job
  try {
    if (fs.fstatSync(fd).size>65536) throw new Error('oversized job')
    job=JSON.parse(fs.readFileSync(fd,'utf8'))
  } finally {fs.closeSync(fd);fs.unlinkSync(input)}
  const root=fs.realpathSync(shared)
  const script=fs.realpathSync(job.script)
  if (job.version!==1 || job.id!==id || !Number.isInteger(job.deadline) || job.deadline<Date.now()/1000 || job.deadline>Date.now()/1000+180 ||
      !script.startsWith(root+path.sep) || path.basename(script)!=='quark-drive.cjs' || !Array.isArray(job.args) || !operations.has(job.args[0]) ||
      job.args.length>64 || !job.args.every(v=>typeof v==='string' && v.length<=8192 && !v.includes('\0')) || !fs.existsSync(active)) throw new Error('invalid job')
  if (job.args[0]==='login' && !(job.args.length===1 || (job.args.length===3 && job.args[1]==='--token'))) throw new Error('invalid login')
  for (let i=1;i<job.args.length;i++) {
    if (job.args[i].startsWith('--')) {
      if (!flags.has(job.args[i]) || (job.args[i]==='--token' && job.args[0]!=='login') || i+1>=job.args.length) throw new Error('invalid flag')
      i++
    } else if (job.args[0]!=='share') throw new Error('unexpected positional value')
  }
  let bytes=0,stderr=0,problem=null
  const chunks=[]
  const child=spawn(process.execPath,[script,...job.args],{cwd:path.dirname(path.dirname(script)),stdio:['ignore','pipe','pipe'],env:{...process.env,NO_COLOR:'1'}})
  let escalation
  const stop = reason => {problem ||= reason;child.kill('SIGTERM');escalation ||= setTimeout(()=>child.kill('SIGKILL'),1500)}
  const timer=setTimeout(()=>stop('timeout'),95000)
  const cancellation=setInterval(()=>{if (!fs.existsSync(active)) stop('cancelled')},400)
  child.stdout.on('data',chunk=>{bytes+=chunk.length;if(bytes>256000)stop('output_limit');else chunks.push(chunk)})
  child.stderr.on('data',chunk=>{stderr+=chunk.length;if(stderr>16000)stop('output_limit')})
  const status=await new Promise(resolve=>{child.on('error',()=>{problem='spawn_failed';resolve(null)});child.on('close',code=>resolve(code))})
  clearTimeout(timer);clearInterval(cancellation);clearTimeout(escalation)
  if (!fs.existsSync(active)) return
  fs.writeFileSync(output,JSON.stringify({version:1,id,success:status===0,error:problem,stdout:Buffer.concat(chunks).toString('utf8')}),{flag:'wx',mode:0o600})
  // This acknowledgement is the only content retained in shell history.
  console.log(JSON.stringify({type:'yuantuan_quark_job',id}))
}
module.exports={runJob}
if(require.main===module) runJob(process.argv[2]).catch(()=>{console.error('Quark worker rejected or could not execute this ticket');process.exitCode=1})
