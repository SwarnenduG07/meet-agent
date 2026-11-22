"""Voice test.
  speak "some text"   -> Kokoro says it out loud, then Whisper checks what was said
  listen [seconds]    -> records your mic, Whisper writes down what you said
"""
import sys, time, subprocess, tempfile
import numpy as np, soundfile as sf
from scipy.signal import resample_poly

def whisper(audio16k):
    import mlx_whisper
    t = time.time()
    r = mlx_whisper.transcribe(audio16k.astype(np.float32), path_or_hf_repo="mlx-community/whisper-large-v3-turbo")
    print(f"[whisper] {time.time()-t:.2f}s -> {r['text'].strip()}")

mode = sys.argv[1] if len(sys.argv) > 1 else "speak"
if mode == "speak":
    from kokoro import KPipeline
    text = sys.argv[2] if len(sys.argv) > 2 else "Hi everyone, I'm Sunny's meet agent. Let me share my screen."
    pipe = KPipeline(lang_code="a", repo_id="hexgrad/Kokoro-82M")
    t = time.time()
    audio = np.concatenate([a.numpy() for _, _, a in pipe(text, voice="af_heart")])
    print(f"[kokoro] {time.time()-t:.2f}s to make {len(audio)/24000:.1f}s of speech")
    wav = tempfile.mktemp(suffix=".wav"); sf.write(wav, audio, 24000)
    subprocess.run(["afplay", wav])
    whisper(resample_poly(audio, 2, 3))
elif mode == "listen":
    import sounddevice as sd
    secs = float(sys.argv[2]) if len(sys.argv) > 2 else 5
    print(f"[mic] talk now for {secs:.0f} seconds...")
    rec = sd.rec(int(secs * 16000), samplerate=16000, channels=1, dtype="float32"); sd.wait()
    print("[mic] done")
    whisper(rec[:, 0])
