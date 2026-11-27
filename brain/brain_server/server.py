"""WebSocket server: one JSON message per text frame, see ../../protocol/messages.json."""
import asyncio
import json
import logging
import threading

import websockets

from .model import Brain, Cancelled

log = logging.getLogger("brain")


class Connection:
    def __init__(self, brain: Brain, ws) -> None:
        self.brain, self.ws = brain, ws
        self.loop = asyncio.get_running_loop()
        self.cancels: dict[int, threading.Event] = {}

    def send_threadsafe(self, msg: dict) -> None:
        asyncio.run_coroutine_threadsafe(self.ws.send(json.dumps(msg)), self.loop)

    async def send(self, msg: dict) -> None:
        await self.ws.send(json.dumps(msg))

    async def run_request(self, msg: dict) -> None:
        rid = msg.get("id")
        cancel = self.cancels.setdefault(rid, threading.Event())
        try:
            if msg["cmd"] == "think":
                action, say = await asyncio.to_thread(
                    self.brain.think,
                    msg["text"],
                    msg.get("state", "LISTENING"),
                    msg.get("allowed", []),
                    msg.get("context", ""),
                    msg.get("image"),
                    cancel,
                    lambda s: self.send_threadsafe({"event": "sentence", "id": rid, "text": s}),
                )
                await self.send({"event": "reply", "id": rid, "action": action, "say": say})
            elif msg["cmd"] == "complete":
                text = await asyncio.to_thread(
                    self.brain.complete, msg["prompt"], int(msg.get("max_tokens", 800)), cancel, msg.get("image")
                )
                await self.send({"event": "text", "id": rid, "text": text})
        except Cancelled:
            await self.send({"event": "cancelled", "id": rid})
        except Exception as e:  # report every failure to Rust, keep serving
            log.exception("request %s failed", rid)
            await self.send({"event": "error", "id": rid, "message": str(e)})
        finally:
            self.cancels.pop(rid, None)

    async def serve(self) -> None:
        await self.send({"event": "ready", "model": self.brain.model_id})
        tasks: set[asyncio.Task] = set()
        async for raw in self.ws:
            try:
                msg = json.loads(raw)
                cmd = msg["cmd"]
            except (ValueError, KeyError) as e:
                await self.send({"event": "error", "id": None, "message": f"bad message: {e}"})
                continue
            if cmd == "cancel":
                if ev := self.cancels.get(msg.get("id")):
                    ev.set()
            elif cmd == "reset":
                self.brain.reset()
            elif cmd in ("think", "complete"):
                t = asyncio.create_task(self.run_request(msg))
                tasks.add(t)
                t.add_done_callback(tasks.discard)
            else:
                await self.send({"event": "error", "id": msg.get("id"), "message": f"unknown cmd {cmd!r}"})


async def serve(brain: Brain, port: int) -> None:
    async def handler(ws):
        log.info("client connected")
        try:
            await Connection(brain, ws).serve()
        except websockets.ConnectionClosed:
            pass
        log.info("client disconnected")

    logging.getLogger("websockets").setLevel(logging.WARNING)
    async with websockets.serve(handler, "127.0.0.1", port, max_size=None):
        print(f"brain-server listening on 127.0.0.1:{port}", flush=True)
        await asyncio.Future()
