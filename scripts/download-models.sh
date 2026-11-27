#!/bin/zsh
# Downloads every meet-agent model into ~/.cache/huggingface/hub.
# Safe to re-run: finished files are skipped and partial ones resume.
export HF_HUB_DISABLE_XET=1   # the Xet transfer was very slow on this network
models=(
  "mlx-community/Qwen3-VL-8B-Instruct-4bit|"          # brain + eyes        ~5.8 GB
  "mlx-community/whisper-large-v3-turbo|"             # speech -> text      ~1.6 GB
  "hexgrad/Kokoro-82M|"                               # text -> speech      ~0.4 GB
  "pipecat-ai/smart-turn-v3|"                         # end of turn         ~0.1 GB
  "onnx-community/silero-vad|"                        # speech detection    ~0.01 GB
  "mlx-community/Qwen3-VL-4B-Instruct-4bit|"          # smaller brain       ~3.1 GB
  "mlx-community/UI-TARS-1.5-7B-4bit|"                # click fallback      ~4.9 GB
  "ResembleAI/chatterbox|ve.safetensors t3_cfg.safetensors s3gen.safetensors tokenizer.json conds.pt"  # voice clone ~3.2 GB
)
for entry in $models; do
  repo=${entry%%|*}; inc=${entry#*|}
  files=(${=inc})
  echo "\n===== $repo ====="
  uvx --from "huggingface_hub[cli]" hf download "$repo" $files
done
echo "\n===== ALL DONE ====="
du -sh ~/.cache/huggingface/hub/models--*

# Formats the Rust agent reads (Phase 2b): Whisper as GGML, Kokoro as ONNX.
echo "\n===== ggerganov/whisper.cpp (large-v3-turbo q8_0) ====="
uvx --from "huggingface_hub[cli]" hf download ggerganov/whisper.cpp ggml-large-v3-turbo-q8_0.bin
echo "\n===== onnx-community/Kokoro-82M-v1.0-ONNX ====="
uvx --from "huggingface_hub[cli]" hf download onnx-community/Kokoro-82M-v1.0-ONNX onnx/model.onnx voices/af_heart.bin tokenizer.json config.json
