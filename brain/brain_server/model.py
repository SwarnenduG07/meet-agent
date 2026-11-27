"""Keeps Qwen3-VL loaded and runs one request at a time."""
import threading
from collections.abc import Callable

from mlx_vlm import load, stream_generate
from mlx_vlm.prompt_utils import apply_chat_template

from .prompts import LOOK_RULES, system_prompt
from .reply import ReplyStream


class Cancelled(Exception):
    pass


class Brain:
    def __init__(self, model_id: str, name: str, owner: str) -> None:
        import transformers

        transformers.logging.set_verbosity_error()
        self.model_id = model_id
        self.model, self.processor = load(model_id)
        self.name, self.owner = name, owner
        self.history: list[dict] = []
        self.lock = threading.Lock()
        # The first generation is slow; pay that cost before announcing `ready`.
        self.complete("Say OK.", 4, threading.Event())

    def _stream(self, messages: list[dict], images: list[str], max_tokens: int, cancel: threading.Event):
        prompt = apply_chat_template(self.processor, self.model.config, messages, num_images=len(images))
        for chunk in stream_generate(
            self.model, self.processor, prompt, image=images or None, max_tokens=max_tokens, temperature=0.3
        ):
            if cancel.is_set():
                raise Cancelled
            yield chunk.text

    def think(
        self,
        text: str,
        state: str,
        allowed: list[str],
        context: str,
        image: str | None,
        cancel: threading.Event,
        on_sentence: Callable[[str], None],
    ) -> tuple[str, str]:
        """Streams sentences through `on_sentence`; returns (action, say)."""
        with self.lock:
            system = system_prompt(state, allowed, context, self.name, self.owner)
            messages = [{"role": "system", "content": system}, *self.history[-8:], {"role": "user", "content": text}]
            reply = ReplyStream()
            for piece in self._stream(messages, [image] if image else [], 200, cancel):
                for sentence in reply.feed(piece):
                    on_sentence(sentence)
            for sentence in reply.finish():
                on_sentence(sentence)
            action = reply.action or "answer"
            say = reply.say.strip()
            if action != "wait":  # a 'wait' in memory turns into a habit
                self.history += [
                    {"role": "user", "content": text},
                    {"role": "assistant", "content": f"ACTION: {action}\nSAY: {say}"},
                ]
            return action, say

    def complete(self, prompt: str, max_tokens: int, cancel: threading.Event, image: str | None = None) -> str:
        """Plain completion without conversation memory (briefs, summaries, looking)."""
        with self.lock:
            system = LOOK_RULES if image else "You are a careful assistant. Follow the instructions exactly."
            messages = [{"role": "system", "content": system}, {"role": "user", "content": prompt}]
            return "".join(self._stream(messages, [image] if image else [], max_tokens, cancel)).strip()

    def reset(self) -> None:
        self.history.clear()
