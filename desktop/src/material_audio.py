"""Bounded, offline-only transcription. Model installation is a separate action."""
import json, math, os, pathlib, sys

def main():
    source, model_dir = map(pathlib.Path, sys.argv[1:3])
    if source.is_symlink() or not source.is_file() or source.stat().st_size > 32*1024*1024:
        raise ValueError('录音不是32MB以内的普通文件')
    if not model_dir.is_dir() or not (model_dir/'model.bin').is_file():
        raise ValueError('本地语音模型尚未安装完整')
    import av
    with av.open(str(source)) as container:
        if not container.streams.audio:
            raise ValueError('文件没有音轨')
        stream=container.streams.audio[0]
        duration=float(stream.duration*stream.time_base) if stream.duration and stream.time_base else (float(container.duration)/av.time_base if container.duration else None)
        if duration is None or not math.isfinite(duration) or duration <= 0 or duration > 15*60:
            raise ValueError('录音时长未知或超过15分钟，请先选择较短片段')
    from faster_whisper import WhisperModel
    model=WhisperModel(str(model_dir),device='cpu',compute_type='int8',cpu_threads=min(os.cpu_count() or 2,4),num_workers=1,local_files_only=True)
    segments, info=model.transcribe(str(source),beam_size=3,vad_filter=True,condition_on_previous_text=False)
    rows=[]
    for segment in segments:
        text=segment.text.strip()
        if text:
            rows.append({'start':round(segment.start,2),'end':round(segment.end,2),'text':text})
        if len(rows)>1000 or sum(len(s['text'].encode('utf-8')) for s in rows)>60*1024:
            raise ValueError('录音转写超过可处理范围，请选择较短片段')
    if not rows:
        raise ValueError('未检测到可确认语音，未生成转写正文')
    body='【本地语音转写，需原录音核对】\n'+'\n'.join(f"[{r['start']:.2f}–{r['end']:.2f}s] {r['text']}" for r in rows)
    print(json.dumps({'engine':'faster-whisper','language':info.language,'duration':duration,'segments':rows,'text':body},ensure_ascii=False))

try:
    main()
except Exception as exc:
    print(str(exc)[:400],file=sys.stderr)
    sys.exit(1)
