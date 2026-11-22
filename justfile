# meet-agent commands. Run `just` to list them.

proto_deps := "--with kokoro --with mlx-vlm --with mlx-whisper --with onnxruntime --with transformers --with soundfile --with scipy --with sounddevice --with torch --with torchvision --with pip"

default:
    @just --list

# Check tools, audio devices, models and the Chrome profile
doctor:
    cargo run -q -p agent -- doctor

# Build everything and run the checks
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace

# Download all models (about 18 GB, safe to re-run)
models:
    ./scripts/download-models.sh

# Python prototype: talk to the agent with your mic (wear headphones)
prototype:
    cd prototype && uv run -q --python 3.12 {{proto_deps}} python voice_loop.py
