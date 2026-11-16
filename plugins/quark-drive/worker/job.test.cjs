const {test}=require('node:test')
const assert=require('node:assert/strict')
const fs=require('node:fs')
const os=require('node:os')
const path=require('node:path')
const {runJob}=require('./job.cjs')
function fixture(t,source) {
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'quark-job-'))
  t.after(()=>fs.rmSync(root,{recursive:true,force:true}))
  const jobs=path.join(root,'quark-service/jobs');fs.mkdirSync(jobs,{recursive:true})
  const script=path.join(root,'quark/scripts/quark-drive.cjs');fs.mkdirSync(path.dirname(script),{recursive:true});fs.writeFileSync(script,source)
  const id='a'.repeat(32)
  const input=path.join(jobs,`${id}.input.json`),output=path.join(jobs,`${id}.output.json`),active=path.join(jobs,`${id}.active`)
  const write=patch=>{fs.writeFileSync(active,'active');fs.writeFileSync(input,JSON.stringify({version:1,id,script,args:['login','--token','private-once'],deadline:Math.floor(Date.now()/1000)+120,...patch}))}
  return {root,id,input,output,active,write}
}
test('only the ticket enters shell output, preserving nonzero business results privately',async t=>{
  const f=fixture(t,"console.log(JSON.stringify({type:'result',code:-106,args:process.argv.slice(2)}));process.exitCode=1")
  f.write({})
  const log=console.log;const lines=[];console.log=value=>lines.push(value)
  try {await runJob(f.id,f.root)}finally{console.log=log}
  assert.equal(fs.existsSync(f.input),false)
  assert.equal(lines.length,1);assert.equal(lines[0].includes('private-once'),false)
  const out=JSON.parse(fs.readFileSync(f.output,'utf8'))
  assert.equal(out.success,false);assert.equal(out.error,null)
  assert.deepEqual(JSON.parse(out.stdout).args,['login','--token','private-once'])
  if(process.platform!=='win32') assert.equal(fs.statSync(f.output).mode & 0o777,0o600)
})
test('invalid operations and expired or cancelled tickets never execute the official program',async t=>{
  const f=fixture(t,"require('fs').writeFileSync('executed','bad')")
  for(const patch of [{args:['delete','all']},{deadline:1},{args:['login','--token','x','--verbose']}]) {
    f.write(patch);await assert.rejects(runJob(f.id,f.root));assert.equal(fs.existsSync(f.output),false)
  }
  f.write({});fs.unlinkSync(f.active);await assert.rejects(runJob(f.id,f.root))
  assert.equal(fs.existsSync(path.join(f.root,'quark/executed')),false)
})
test('cancellation kills the running CLI without retaining output',async t=>{
  const f=fixture(t,"setInterval(()=>{},1000)");f.write({})
  const done=runJob(f.id,f.root)
  setTimeout(()=>fs.unlinkSync(f.active),100)
  await done;assert.equal(fs.existsSync(f.output),false)
})
test('output limits stop the child and return a bounded private failure',async t=>{
  const f=fixture(t,"process.stdout.write('x'.repeat(400000));setInterval(()=>{},1000)");f.write({})
  await runJob(f.id,f.root)
  const out=JSON.parse(fs.readFileSync(f.output,'utf8'))
  assert.equal(out.error,'output_limit');assert.ok(Buffer.byteLength(out.stdout)<=256000)
})
