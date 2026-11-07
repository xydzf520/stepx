#!/usr/bin/env python3
"""Install optional offline speech recognition into StepX's private runtime.

Uses the official faster-whisper package and model repository. No sudo, system
Python changes, automatic phone reads or cloud transcription. Transcription only
uses the completed local model directory; this installer is the network step.
"""
import argparse, json, os, pathlib, subprocess, sys, tempfile, venv

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--model',choices=['tiny','base','small'],default='small')
    p.add_argument('--runtime-dir',type=pathlib.Path)
    p.add_argument('--model-dir',type=pathlib.Path)
    args=p.parse_args()
    home=pathlib.Path.home()
    runtime=(args.runtime_dir or home/'.local/share/stepx-companion/material-asr').resolve()
    model=(args.model_dir or home/'.cache/stepx-companion/material-asr'/args.model).resolve()
    for d in [runtime,model]:
        if d.is_symlink():raise ValueError('安装目录不能是符号链接')
        d.mkdir(parents=True,exist_ok=True,mode=0o700)
        if os.name!='nt':d.chmod(0o700)
    environment=runtime/'venv'
    python=environment/('Scripts/python.exe' if os.name=='nt' else 'bin/python3')
    if not python.is_file():venv.EnvBuilder(with_pip=True).create(environment)
    # faster-whisper 1.2.1 still passes PyAV's metadata_errors parameter, removed
    # in PyAV 19. Pin the decoder verified by the shipped offline runner.
    subprocess.run([str(python),'-m','pip','install','--disable-pip-version-check','faster-whisper==1.2.1','av==16.0.1'],check=True)
    code='from faster_whisper.utils import download_model;import sys;download_model(sys.argv[1],output_dir=sys.argv[2])'
    subprocess.run([str(python),'-I','-c',code,args.model,str(model)],check=True)
    if not (model/'model.bin').is_file():raise ValueError('模型下载尚未完成，未启用本地转写')
    config_root=pathlib.Path(os.environ.get('STEPX_CONFIG_DIR') or os.environ.get('XDG_CONFIG_HOME') or os.environ.get('APPDATA') or home/'.config')/'stepx-companion'
    config_root.mkdir(parents=True,exist_ok=True,mode=0o700)
    config=config_root/'material-asr.json'
    if config.is_symlink():raise ValueError('配置文件不能是符号链接')
    payload={'schema':1,'engine':'faster-whisper','version':'1.2.1','python':str(python),'model_dir':str(model),'model':args.model,'device':'cpu','compute_type':'int8','local_only':True}
    with tempfile.NamedTemporaryFile('w',dir=config_root,encoding='utf-8',delete=False) as f:
        json.dump(payload,f,ensure_ascii=False,indent=2);f.flush();os.fsync(f.fileno());tmp=pathlib.Path(f.name)
    tmp.replace(config)
    print(json.dumps({'ready':True,'config':str(config),'model':args.model,'engine':'faster-whisper'},ensure_ascii=False))

if __name__=='__main__':main()
