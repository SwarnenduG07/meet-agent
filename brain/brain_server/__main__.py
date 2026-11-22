"""Entry point: `uv run brain-server --port 8790`. The server itself arrives in Phase 2b."""
import argparse


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--port", type=int, default=8790)
    ap.add_argument("--model", default="mlx-community/Qwen3-VL-8B-Instruct-4bit")
    args = ap.parse_args()
    raise SystemExit(f"brain-server not built yet (Phase 2b). Would serve {args.model} on 127.0.0.1:{args.port}.")


if __name__ == "__main__":
    main()
