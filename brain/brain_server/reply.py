"""Turns the model's streamed text into an action plus whole sentences, so speech
can start after the first sentence instead of after the full answer."""
import re

SENTENCE_END = re.compile(r"(?<=[.!?])\s+")


class ReplyStream:
    """Feed text chunks; collect finished sentences of SAY as they complete."""

    def __init__(self) -> None:
        self.raw = ""
        self.action: str | None = None
        self.say = ""
        self._said_upto = 0

    def feed(self, chunk: str) -> list[str]:
        self.raw += chunk
        self._parse()
        return self._new_sentences(final=False)

    def finish(self) -> list[str]:
        self._parse()
        if self.action is None and not self.say:
            # The model ignored the format: speak what it wrote.
            self.action, self.say = "answer", self.raw.strip()
        return self._new_sentences(final=True)

    def _parse(self) -> None:
        m = re.search(r"ACTION:\s*([a-z_]+)", self.raw)
        if m and "\n" in self.raw[m.end():] + ("\n" if "SAY:" in self.raw else ""):
            self.action = m.group(1)
        s = re.search(r"SAY:\s*", self.raw)
        if s:
            self.say = self.raw[s.end():]

    def _new_sentences(self, final: bool) -> list[str]:
        pending = self.say[self._said_upto:]
        parts = SENTENCE_END.split(pending)
        done = parts if final else parts[:-1]
        out = []
        for p in done:
            self._said_upto += len(p)
            # skip the whitespace that followed this sentence
            while self._said_upto < len(self.say) and self.say[self._said_upto].isspace():
                self._said_upto += 1
            if p.strip():
                out.append(p.strip())
        return out
