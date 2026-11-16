"""Real Docker/OpenCode execution; no LLM calls. Optional official SDK OAuth probe.

Uses an isolated directory/container and a loopback-only random test port.
Never mount production credentials or accept an account authorization code.
"""
import argparse
import base64
import hashlib
import io
import json
import os
import pathlib
import re
import secrets
import shutil
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--image', required=True)
    parser.add_argument('--official-sdk', action='store_true')
    args = parser.parse_args()
    root = pathlib.Path(tempfile.mkdtemp(prefix='yuantuan-quark-worker-')).resolve()
    name = 'yuantuan-quark-worker-smoke-' + secrets.token_hex(5)
    created = False
    try:
        root.chmod(0o755)
        service = root / 'quark-service'
        jobs = service / 'jobs'
        jobs.mkdir(parents=True, mode=0o700)
        password = secrets.token_urlsafe(32)
        (service / 'password').write_text(password)
        (service / 'password').chmod(0o600)
        script = root / 'quark/scripts/quark-drive.cjs'
        script.parent.mkdir(parents=True)
        # Native OpenCode must supply its real context. The adapter does not set it.
        script.write_text("""
const opencode=process.env.OPENCODE!==undefined && process.env.OPENCODE_PID!==undefined;
if(!opencode) {console.log(JSON.stringify({type:'result',code:-104}));process.exit(1)}
if(process.argv[2]==='resolve-agent') console.log('QK_AGENT_ID=opencode');
else {console.log(JSON.stringify({type:'result',code:0,fixture:'private fixture result',args:process.argv.slice(2)}));}
""")
        uid, gid = os.getuid(), os.getgid()
        subprocess.check_output([
            'docker','run','-d','--name',name,'--user',f'{uid}:{gid}',
            '--memory','768m','--cpus','1','--pids-limit','128',
            '--cap-drop','ALL','--security-opt','no-new-privileges:true',
            '--env','HOME=/tmp/quark-worker-home','--workdir','/tmp',
            '--mount',f'type=bind,source={root},target=/app/data/connectors',
            '-p','127.0.0.1::4096',args.image,
        ], stderr=subprocess.STDOUT)
        created = True
        port = subprocess.check_output(['docker','port',name,'4096/tcp'],text=True).strip().rsplit(':',1)[1]
        base = f'http://127.0.0.1:{port}'
        authorization = 'Basic ' + base64.b64encode(('opencode:' + password).encode()).decode()

        def request(path, method='GET', body=None, authenticated=True):
            headers = {'Content-Type':'application/json'}
            if authenticated:
                headers['Authorization'] = authorization
            req = urllib.request.Request(base+path, method=method, headers=headers,
                data=json.dumps(body).encode() if body is not None else None)
            with urllib.request.urlopen(req,timeout=110) as response:
                return json.load(response)

        deadline = time.monotonic()+75
        while True:
            try:
                # Short readiness timeout; never leave an unauthenticated wget hanging.
                req = urllib.request.Request(base+'/global/health',headers={'Authorization':authorization})
                with urllib.request.urlopen(req,timeout=2) as response:
                    health = json.load(response)
                if health.get('healthy'):
                    break
            except (OSError, ValueError):
                pass
            assert time.monotonic()<deadline, 'worker readiness timed out'
            time.sleep(1)
        try:
            request('/session', 'POST', {}, authenticated=False)
            raise AssertionError('worker admitted an unauthenticated operator request')
        except urllib.error.HTTPError as error:
            assert error.code == 401

        def execute(argv):
            ticket = secrets.token_hex(16)
            input_path = jobs / (ticket+'.input.json')
            active = jobs / (ticket+'.active')
            output_path = jobs / (ticket+'.output.json')
            active.write_text('active')
            input_path.write_text(json.dumps({'version':1,'id':ticket,
                'script':'/app/data/connectors/quark/scripts/quark-drive.cjs',
                'args':argv,'deadline':int(time.time())+120}))
            input_path.chmod(0o600)
            session = request('/session','POST',{'title':'Isolated Quark tool check'})['id']
            try:
                response=request('/session/'+session+'/shell','POST',{'agent':'build',
                    'command':'node /opt/quark-tools/job.cjs '+ticket})
                assert output_path.is_file(), 'worker omitted actual CLI receipt'
                result=json.loads(output_path.read_text())
                assert result['id']==ticket and result['error'] is None
                assert not input_path.exists()
                tokens=response['info']['tokens']
                assert tokens['input']==0 and tokens['output']==0, 'worker used a model turn'
                history=json.dumps(request('/session/'+session+'/message'))
                assert 'private-one-time-code' not in history
                assert 'private fixture result' not in history
                assert 'QK_AGENT_ID=' not in history
                assert 'page_code=' not in history, 'OAuth URL entered worker history'
                return result
            finally:
                request('/session/'+session,'DELETE')
                for path in [active,input_path,output_path]:
                    path.unlink(missing_ok=True)

        preflight=execute(['resolve-agent'])
        assert preflight['success'] and preflight['stdout'].strip()=='QK_AGENT_ID=opencode', 'real runtime markers missing'
        receipt=execute(['login','--token','private-one-time-code'])
        assert json.loads(receipt['stdout'])['args']==['login','--token','private-one-time-code']
        report={'runtime':health.get('version'),'native_execution':'passed','model_tokens':0,
            'private_receipts':'passed','unauthenticated_operator':'rejected'}
        if args.official_sdk:
            payload=urllib.request.urlopen('https://pdds.quark.cn/download/stfile/eekkigineefiegmgo/quarkclouddrive-1.0.25.zip',timeout=30).read(8*1024*1024+1)
            assert hashlib.sha256(payload).hexdigest()=='56c34e9a9313ae29d39475b4ff9da9e92fbfd59699bf5843e3382e54434fffb8'
            sdk=root/'quark'
            with zipfile.ZipFile(io.BytesIO(payload)) as archive:
                for entry in archive.infolist():
                    target=(sdk/entry.filename).resolve()
                    assert target.is_relative_to(sdk) and entry.file_size<=8*1024*1024
                    if entry.is_dir():target.mkdir(parents=True,exist_ok=True)
                    else:target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(archive.read(entry))
            assert execute(['resolve-agent'])['stdout'].strip()=='QK_AGENT_ID=opencode'
            result=execute(['login'])
            rows=[]
            for line in result['stdout'].splitlines():
                try: rows.append(json.loads(line))
                except ValueError: pass
            final=next(row for row in rows if row.get('type')=='result')
            links=re.findall(r'https://[^\s<>`]+', final.get('msg',''))
            official=[urllib.parse.urlparse(link) for link in links if urllib.parse.urlparse(link).hostname=='pan.quark.cn']
            assert final['code']==-106 and any('client_id' in urllib.parse.parse_qs(link.query) for link in official), 'official OAuth link not obtained'
            report.update(official_sdk='1.0.25',oauth_link='obtained',oauth_host='pan.quark.cn',account_consent='not_performed')
        assert not list(jobs.iterdir()), 'ephemeral jobs left behind'
        report['resource']=subprocess.check_output(['docker','stats','--no-stream','--format','{{.MemUsage}}',name],text=True).strip()
        print(json.dumps(report),flush=True)
    finally:
        if created:
            subprocess.run(['docker','rm','-f',name],check=True,stdout=subprocess.DEVNULL)
        assert root.name.startswith('yuantuan-quark-worker-') and root.parent==pathlib.Path(tempfile.gettempdir()).resolve()
        shutil.rmtree(root)

if __name__=='__main__':
    main()
