"""Working Python prototype of the voice loop (Phase 2). Reference for the Rust port.

mic -> Silero VAD -> Smart Turn -> Whisper -> Qwen3-VL brain -> Kokoro -> speakers.

Live:      python voice_loop.py
Self-test: python voice_loop.py --file question.wav --out answer.wav
Brief:     ../briefs/example.md (override with --brief)
"""
import argparse, glob, json, os, queue, re, time
import numpy as np, onnxruntime as ort, soundfile as sf
from scipy.signal import resample_poly

SR = 16000
CHUNK = 512                      # 32 ms per VAD step
HUB = os.path.expanduser("~/.cache/huggingface/hub")
HERE = os.path.dirname(os.path.abspath(__file__))

def log(stage, msg): print(f"[{time.strftime('%H:%M:%S')}] [{stage:<7}] {msg}", flush=True)

# ---------- ears: speech detection ----------
class VAD:
    def __init__(self):
        self.s = ort.InferenceSession(glob.glob(f"{HUB}/models--onnx-community--silero-vad/snapshots/*/onnx/model.onnx")[0])
        self.reset()
    def reset(self):
        self.state = np.zeros((2, 1, 128), np.float32); self.ctx = np.zeros(64, np.float32)
    def prob(self, chunk):
        x = np.concatenate([self.ctx, chunk])[None].astype(np.float32); self.ctx = chunk[-64:]
        out, self.state = self.s.run(None, {"input": x, "state": self.state, "sr": np.array(SR, np.int64)})
        return float(out[0][0])

# ---------- ears: has the speaker finished? ----------
class SmartTurn:
    def __init__(self):
        from transformers import WhisperFeatureExtractor
        self.fe = WhisperFeatureExtractor(chunk_length=8)
        self.s = ort.InferenceSession(glob.glob(f"{HUB}/models--pipecat-ai--smart-turn-v3/snapshots/*/smart-turn-v3.2-cpu.onnx")[0])
    def done_prob(self, audio):
        audio = audio[-8 * SR:]
        f = self.fe(audio, sampling_rate=SR, return_tensors="np", padding="max_length",
                    max_length=8 * SR, truncation=True, do_normalize=True).input_features.astype(np.float32)
        return float(self.s.run(None, {"input_features": f})[0][0][0])

# ---------- brain ----------
class Brain:
    def __init__(self, brief_path):
        from mlx_vlm import load
        import transformers; transformers.logging.set_verbosity_error()
        self.model, self.proc = load("mlx-community/Qwen3-VL-8B-Instruct-4bit")
        self.brief = open(brief_path).read()
        self.history = []
    def system(self):
        return (
            "You are Sunny's meet agent, live in a Google Meet call and speaking out loud for Sunny.\n"
            f"Today is {time.strftime('%A, %d %B %Y')}, the time is {time.strftime('%H:%M')}.\n"
            "Reply with ONLY a JSON object: {\"action\": \"answer\" | \"defer_to_sunny\" | \"wait\", \"say\": \"...\"}.\n"
            "Rules:\n"
            "- Greetings and small talk (hello, how are you, what is your name, what is the date): answer briefly and friendly.\n"
            "- Questions about the project: answer only with facts from the brief.\n"
            "- Project questions the brief does not cover, or do-not-discuss topics: defer_to_sunny, say Sunny will follow up.\n"
            "- wait (say \"\") ONLY for an unfinished fragment like \"so, um\" or \"quick question\". Never wait on a real question or request.\n"
            "- say: 1-2 short natural spoken sentences, no markdown, no lists.\n\n"
            "BRIEF:\n" + self.brief)
    def _ask(self, question, extra=""):
        from mlx_vlm import generate
        from mlx_vlm.prompt_utils import apply_chat_template
        msgs = [{"role": "system", "content": self.system() + extra}] + self.history[-6:] + [{"role": "user", "content": question}]
        prompt = apply_chat_template(self.proc, self.model.config, msgs, num_images=0)
        r = generate(self.model, self.proc, prompt, max_tokens=160, verbose=False)
        text = getattr(r, "text", r)
        m = re.search(r"\{.*\}", text, re.S)
        try: return json.loads(m.group(0))
        except Exception: return {"action": "answer", "say": text.strip()}
    def reply(self, question):
        out = self._ask(question)
        # safety net: a real question must never be ignored
        if out.get("action") == "wait" and (question.rstrip().endswith("?") or len(question.split()) >= 5):
            out = self._ask(question, "\nThe person asked a real question and is waiting. Do not use wait.")
        if out.get("action") != "wait":           # keep 'wait' out of memory so it doesn't become a habit
            self.history += [{"role": "user", "content": question}, {"role": "assistant", "content": json.dumps(out)}]
        return out

# ---------- mouth ----------
class Voice:
    def __init__(self):
        from kokoro import KPipeline
        self.pipe = KPipeline(lang_code="a", repo_id="hexgrad/Kokoro-82M")
    def speak(self, text, out_path=None):
        parts, first = [], None
        t = time.time()
        for sent in [x for x in re.split(r"(?<=[.!?])\s+", text.strip()) if x]:   # speak sentence by sentence
            for _, _, a in self.pipe(sent, voice="af_heart"):
                a = a.numpy(); parts.append(a)
                if first is None: first = time.time() - t; log("kokoro", f"first audio after {first:.2f}s")
                if not out_path:
                    import sounddevice as sd; sd.play(a, 24000); sd.wait()
        if out_path: sf.write(out_path, np.concatenate(parts), 24000)
        return first or 0.0

def transcribe(audio):
    import mlx_whisper
    r = mlx_whisper.transcribe(audio.astype(np.float32), path_or_hf_repo="mlx-community/whisper-large-v3-turbo",
                               initial_prompt="Google Meet, meet agent, Sunny, Flowline, Rust API, Next.js, Supabase, Nango.")
    return r["text"].strip()

# ---------- the loop ----------
def chunks_from_file(path):
    a, sr = sf.read(path, dtype="float32")
    if a.ndim > 1: a = a.mean(1)
    if sr != SR: a = resample_poly(a, SR, sr).astype(np.float32)
    a = np.concatenate([a, np.zeros(3 * SR, np.float32)])   # trailing silence so the turn can end
    for i in range(0, len(a) - CHUNK, CHUNK): yield a[i:i + CHUNK]

def chunks_from_mic(q):
    while True: yield q.get()

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--file"); ap.add_argument("--out")
    ap.add_argument("--brief", default=os.path.join(HERE, "..", "briefs", "example.md"))
    args = ap.parse_args()
    t = time.time()
    log("startup", "loading models (stay in memory from now on)...")
    vad, turn, brain, voice = VAD(), SmartTurn(), Brain(args.brief), Voice()
    transcribe(np.zeros(SR, np.float32))                     # warm Whisper
    list(voice.pipe("Ready.", voice="af_heart"))             # warm Kokoro
    log("startup", f"ready in {time.time()-t:.1f}s")

    mic_q = None
    if args.file:
        source = chunks_from_file(args.file)
    else:
        import sounddevice as sd
        mic_q = queue.Queue()
        stream = sd.InputStream(samplerate=SR, channels=1, dtype="float32", blocksize=CHUNK,
                                callback=lambda d, *_: mic_q.put(d[:, 0].copy()))
        stream.start(); source = chunks_from_mic(mic_q)
        log("listen", "ask me something about Flowline (Ctrl+C to quit)")

    speech, talking, silent_ms, pending, pending_t = [], False, 0, "", 0.0
    for chunk in source:
        p = vad.prob(chunk)
        if p > 0.5:
            if not talking: log("vad", "speech started"); talking = True
            silent_ms = 0
        if not talking:
            continue
        speech.append(chunk)
        if p <= 0.5: silent_ms += 32
        # after a short pause, ask Smart Turn whether they're actually done
        if silent_ms not in (512, 800, 1088) and silent_ms < 2000:
            continue
        audio = np.concatenate(speech)
        dp = turn.done_prob(audio)
        if dp <= 0.5 and silent_ms < 2000:
            log("turn", f"pause {silent_ms}ms but not finished yet (done={dp:.2f}), still listening")
            continue
        t_end = time.time()
        log("turn", f"finished speaking (done={dp:.2f}, pause {silent_ms}ms, {len(audio)/SR:.1f}s of speech)")
        t = time.time(); q = transcribe(audio); log("whisper", f"{time.time()-t:.2f}s  heard: {q}")
        if time.time() - pending_t > 10: pending = ""         # an old fragment is not part of this question
        q = (pending + " " + q).strip(); pending = ""
        if q:
            t = time.time(); out = brain.reply(q); log("brain", f"{time.time()-t:.2f}s  {out}")
            if out.get("action") == "wait" or not out.get("say"):
                pending, pending_t = q, time.time(); log("brain", "unfinished sentence, waiting for the rest")
            else:
                t_brain_done = time.time()
                first = voice.speak(out["say"], args.out)
                log("total", f"{t_brain_done - t_end + first:.2f}s from end of question to first audio")
        speech, talking, silent_ms = [], False, 0; vad.reset()
        if mic_q:                                  # drop what the mic heard while we talked
            while not mic_q.empty(): mic_q.get_nowait()
            log("listen", "your turn")

if __name__ == "__main__":
    try: main()
    except KeyboardInterrupt: print("\nbye")
