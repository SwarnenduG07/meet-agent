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

# Open the agent's Chrome profile to sign in to its Google account once
login:
    cargo run -q -p agent -- login

# Talk to the agent with your mic (Rust voice loop; wear headphones for --barge-in)
listen *args:
    cargo run -q --release -p agent -- listen {{args}}

# Ask the brain one question (no audio)
ask question:
    cargo run -q --release -p agent -- ask "{{question}}"

# Join a meeting and talk (t = take over, s = share, l = leave)
talk link *args:
    cargo run -q --release -p agent -- talk "{{link}}" {{args}}

# Run a whole meeting from a brief (prepare, join, present, answer, summary)
run brief *args:
    cargo run -q --release -p agent -- run "{{brief}}" {{args}}

# Practise a brief locally: same flow, no Meet
rehearse brief *args:
    cargo run -q --release -p agent -- run "{{brief}}" --rehearse {{args}}

# Find something on screen and ring it, e.g. just point "Orchestrator"
point target *args:
    cargo run -q --release -p agent -- point "{{target}}" {{args}}
