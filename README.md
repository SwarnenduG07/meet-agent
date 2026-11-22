# meet-agent

A local agent that joins Google Meet for you, shares your screen, presents from a
brief and answers questions by voice. Runs on your Mac with open-source models.

Architecture: https://claude.ai/artifact/DR3FLfmkbyetLtKS8vJuNi

## Layout

| Path | Language | What |
|---|---|---|
| `crates/agent` | Rust | `meet-agent` CLI, orchestrator (states, rules, actions, log) |
| `crates/audio` | Rust | BlackHole in/out, Silero VAD, Smart Turn |
| `crates/speech` | Rust | Whisper (speech → text), Kokoro (text → speech) |
| `crates/meet` | Rust | Google Meet in real Chrome over CDP |
| `crates/desktop` | Rust | Window control, screenshots, spotlight |
| `crates/brain-client` | Rust | Starts and talks to the Python brain |
| `brain/` | Python | Brain server: Qwen3-VL on MLX, answers `think()` |
| `protocol/messages.json` | JSON | The Rust ↔ Python message contract |
| `config/` | TOML | Settings and Meet button labels |
| `briefs/` | Markdown | One brief per meeting |
| `prototype/` | Python | Working voice loop from Phase 2, reference for the port |

Models live in `~/.cache/huggingface/hub`. The agent's Chrome profile lives in
`~/meet-agent-chrome`. Neither is in this repo.

## Setup

Needs Rust, `uv`, `just` and Google Chrome.

```bash
brew install blackhole-2ch blackhole-16ch espeak-ng   # then reboot
just models
just doctor
```

## Commands

```bash
just doctor       # what is installed, what is missing
just check        # fmt, clippy, tests
just prototype    # the Python voice loop with your mic
```
