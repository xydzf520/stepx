#!/usr/bin/env python3
"""Build a portable package from the native release binary and verified upstream tools."""
import argparse, hashlib, os, platform, plistlib, shutil, subprocess, sys, tarfile, tempfile, urllib.request, zipfile
from pathlib import Path
from public_release_policy import audit_bundle, content_problems, manifest, runtime_plan, safe_read

ROOT = Path(__file__).resolve().parents[1]
TOOLS = {
    'linux-x86_64': ('scrcpy-linux-x86_64-v5.0.1.tar.gz', '9f969d30cc574816077edecda65719c1c70b0aee1df1ba5fa00779ac07b8fd6e'),
    'macos-arm64': ('scrcpy-macos-aarch64-v5.0.1.tar.gz', '33611e51977a8289e2e124b220f727895181ef2eb9060e0987a4dc2db9cf0d6b'),
    'macos-x86_64': ('scrcpy-macos-x86_64-v5.0.1.tar.gz', '31a5467a9e907f162b093267cbbab1276b15c6e709c2ea8159566a8d165beb50'),
    'windows-x86_64': ('scrcpy-win64-v5.0.1.zip', 'b12a2c4ee8be317422451fc7dcf8ee20a71b5ea7ef9ad73ddd825a25316227a5'),
}

def main():
    p=argparse.ArgumentParser();p.add_argument('--platform',choices=TOOLS,required=True);p.add_argument('--version',required=True);a=p.parse_args()
    if not all(c.isalnum() or c in '.-_' for c in a.version):p.error('Invalid version')
    import tomllib
    if tomllib.loads((ROOT/'desktop/Cargo.toml').read_text())['package']['version']!=a.version: p.error('Package version must match Cargo.toml')
    windows=a.platform.startswith('windows');mac=a.platform.startswith('macos')
    host_os={'Darwin':'macos','Windows':'windows','Linux':'linux'}.get(platform.system())
    host_arch={'AMD64':'x86_64','aarch64':'arm64'}.get(platform.machine(),platform.machine())
    if a.platform != f'{host_os}-{host_arch}': p.error('Use a native runner for the requested platform; cross-built packages cannot be runtime-verified here')
    policy=manifest(ROOT)
    runtime_plan(ROOT)  # fail closed before staging any resource
    documents={rel:safe_read(ROOT,rel) for rel in policy['package_documents']}
    issues=[issue for rel,data in documents.items() for issue in content_problems(rel,data,policy)]
    if issues: raise RuntimeError('\n'.join(issues))
    dist=ROOT/'dist';dist.mkdir(exist_ok=True)
    name=f'StepX-Companion-{a.version}-{a.platform}'
    with tempfile.TemporaryDirectory(prefix='stepx-package-') as td:
        stage=Path(td);base=stage/name;base.mkdir()
        if mac:
            bundle=base/'StepX Companion.app'/'Contents';binary_dir=bundle/'MacOS';resource_dir=bundle/'Resources'
            binary_dir.mkdir(parents=True);resource_dir.mkdir()
            info={'CFBundleName':'StepX Companion','CFBundleDisplayName':'StepX Companion','CFBundleIdentifier':'io.github.xydzf520.stepx','CFBundleExecutable':'stepx-companion','CFBundlePackageType':'APPL','CFBundleShortVersionString':a.version,'CFBundleVersion':a.version,'NSHighResolutionCapable':True,'NSMicrophoneUsageDescription':'用于语音输入','NSCameraUsageDescription':'用于扫码连接'}
            info['NSLocalNetworkUsageDescription']='用于在局域网发现并连接您授权的手机。'
            info['NSBonjourServices']=['_adb-tls-pairing._tcp','_adb-tls-connect._tcp','_adb._tcp']
            iconset=stage/'StepX.iconset';iconset.mkdir()
            for size in [16,32,128,256,512]:
                for scale in [1,2]:
                    suffix='@2x' if scale==2 else ''
                    subprocess.run(['sips','-z',str(size*scale),str(size*scale),str(ROOT/'desktop/assets/stepx_companion_icon.png'),'--out',str(iconset/f'icon_{size}x{size}{suffix}.png')],check=True,stdout=subprocess.DEVNULL)
            subprocess.run(['iconutil','-c','icns',str(iconset),'-o',str(resource_dir/'StepX.icns')],check=True)
            info['CFBundleIconFile']='StepX.icns'
            (bundle/'Info.plist').write_bytes(plistlib.dumps(info))
        else:binary_dir=resource_dir=base
        binary_name='stepx-companion'+('.exe' if windows else '')
        shutil.copy2(ROOT/'desktop'/'target'/'release'/binary_name,binary_dir/binary_name)
        subprocess.run([sys.executable,str(ROOT/'scripts/stage-release-resources.py'),str(ROOT),str(resource_dir)],check=True)
        for rel,data in documents.items():
            target=resource_dir/rel;target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(data)
        filename,digest=TOOLS[a.platform];download=stage/filename
        urllib.request.urlretrieve('https://github.com/Genymobile/scrcpy/releases/download/v5.0.1/'+filename,download)
        if hashlib.sha256(download.read_bytes()).hexdigest()!=digest:raise RuntimeError('Tool archive SHA256 mismatch')
        unpack=stage/'upstream';unpack.mkdir()
        if filename.endswith('.zip'):
            with zipfile.ZipFile(download) as z:z.extractall(unpack)
        else:
            with tarfile.open(download) as t:t.extractall(unpack,filter='data')
        roots=list(unpack.iterdir());source=roots[0] if len(roots)==1 and roots[0].is_dir() else unpack
        tools_dir=resource_dir/'tools/bin';shutil.copytree(source,tools_dir)
        subprocess.run([sys.executable,str(ROOT/'scripts/package-runtime.py'),str(resource_dir),*(['--windows'] if windows else [])],check=True)
        tool_env=os.environ.copy();tool_env['LD_LIBRARY_PATH']=str(tools_dir)+os.pathsep+tool_env.get('LD_LIBRARY_PATH','')
        for executable,arg in [(binary_dir/binary_name,'--version'),(tools_dir/('adb.exe' if windows else 'adb'),'version'),(tools_dir/('scrcpy.exe' if windows else 'scrcpy'),'--version')]:
            subprocess.run([str(executable),arg],check=True,env=tool_env,timeout=30)
        python=resource_dir/'runtime/python/python.exe' if windows else resource_dir/'runtime/python/bin/python3'
        subprocess.run([str(python),'-X','utf8',str(ROOT/'scripts/verify-packaged-runtime.py'),str(resource_dir)],check=True,timeout=180)
        if mac:
            subprocess.run(['codesign','--force','--deep','--sign','-',str(base/'StepX Companion.app')],check=True)
            subprocess.run([str(binary_dir/binary_name),'--check-runtime'],check=True,timeout=45)
            subprocess.run(['codesign','--verify','--deep','--strict',str(base/'StepX Companion.app')],check=True)
        issues=audit_bundle(resource_dir)
        if issues: raise RuntimeError('Package privacy verification failed:\n'+'\n'.join(issues))
        if a.platform.startswith('linux'):
            output=dist/(name+'.tar.gz')
            with tarfile.open(output,'w:gz') as t:t.add(base,arcname=name)
        else:
            output=dist/(name+'.zip')
            if mac:subprocess.run(['ditto','-c','-k','--sequesterRsrc','--keepParent',str(base),str(output)],check=True)
            else:shutil.make_archive(str(dist/name),'zip',stage,name)
        (dist/(output.name+'.sha256')).write_text(hashlib.sha256(output.read_bytes()).hexdigest()+'  '+output.name+'\n')
        print(output)
if __name__=='__main__':main()
