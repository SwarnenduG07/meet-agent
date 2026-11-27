"""Entry point: `uv run brain-server --port 8790`. Started by the Rust agent."""
import argparse
import asyncio
import logging
import time


def watch_parent(pid: int) -> None:
    """Exits when the Rust agent is gone, so a killed agent never leaves the
    model (about 6 GB) running in the background."""
    import os
    import threading

    def loop() -> None:
        while True:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                os._exit(0)
            time.sleep(1)

    threading.Thread(target=loop, daemon=True).start()


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--port", type=int, default=8790)
    ap.add_argument("--model", default="mlx-community/Qwen3-VL-8B-Instruct-4bit")
    ap.add_argument("--name", default="Sunny's meet agent")
    ap.add_argument("--owner", default="Sunny")
    ap.add_argument("--parent-pid", type=int, help="exit when this process is gone")
    args = ap.parse_args()
    logging.basicConfig(level=logging.INFO, format="[brain] %(message)s")
    logging.getLogger("httpx").setLevel(logging.WARNING)
    if args.parent_pid:
        watch_parent(args.parent_pid)

    from .model import Brain
    from .server import serve

    t = time.time()
    brain = Brain(args.model, args.name, args.owner)
    logging.info("model loaded in %.1fs", time.time() - t)
    asyncio.run(serve(brain, args.port))


if __name__ == "__main__":
    main()
