"""Voice command grammar for DravenIQ — deterministic, offline, no LLM.

``parse(text) -> list[Action]`` turns one transcribed utterance into the
actions the daemon executes. Grammar first: a mis-heard command must do
nothing rather than something surprising, so every pattern is anchored and
anything that matches no pattern only becomes dictation when it is long
enough to be a sentence (and dictation never presses Enter by itself).

Actions are (verb, payload) tuples:
  ("focus", None)                     bring DravenIQ to the front
  ("new_tab", {"n": 3, "agent": "claude"})
  ("open_plan", None)
  ("close_tab", None)
  ("select_tab", "next" | "prev")
  ("settings", None)
  ("hide", None)
  ("pause", None) / ("listen", None)  stop / resume wake-word handling
  ("enter", None)                     press Enter in the active pane
  ("dictate", "text")                 type text, no Enter
  ("unknown", "text")
"""
import re

# every way whisper has spelled the name so far, longest first
WAKE_WORDS = ("hey draven", "hey drayven", "hey darren", "hey driven", "hey dravin",
              "hey draven iq", "hey devin", "hey daven", "ok draven", "draven iq", "draven", "drayven", "darren", "devin", "daven",
              "driven", "dravin", "hey jarvis", "hey marvin", "hey mycroft", "jarvis")

NUMBERS = {"a": 1, "an": 1, "one": 1, "two": 2, "to": 2, "too": 2,
           "three": 3, "four": 4, "for": 4, "five": 5, "six": 6,
           "seven": 7, "eight": 8, "nine": 9, "ten": 10}

AGENTS = ("claude", "codex", "hermes")


NAME_TOKENS = {"hey", "ok", "okay", "hi", "draven", "drayven", "dravin", "darren", "darwin",
               "driven", "devin", "daven", "deven", "jarvis", "marvin", "mycroft", "iq", "oh", "man"}


def normalize(text):
    """Lower-case, strip punctuation, drop the wake phrase however whisper
    spelled it. Returns "" when the utterance was ONLY the name (the wake
    fired early and the real command is still coming)."""
    t = text.lower().strip()
    t = re.sub(r"[^\w\s']", " ", t)
    t = re.sub(r"\s+", " ", t).strip()
    for w in WAKE_WORDS:
        if t.startswith(w + " "):
            t = t[len(w) + 1:]
            break
        if t == w:
            return ""
    words = t.split()
    while words and words[0] in NAME_TOKENS:
        words.pop(0)
    return " ".join(words).strip(" ,")


def _count(word):
    if word is None:
        return 1
    if word.isdigit():
        return max(1, min(int(word), 10))
    return NUMBERS.get(word, 1)


_TABS = re.compile(
    r"^(?:open|new|create|make|spawn|start)(?: up)?(?: me)?"
    r"(?: (?P<n>\d+|a|an|one|two|to|too|three|four|for|five|six|seven|eight|nine|ten))?"
    r"(?: new)?(?: (?P<agent>claude|codex|hermes))?"
    r"(?: mini| little| small)? tabs?(?: here| in this project)?$")

_PATTERNS = [
    (re.compile(r"^(?:open|show|focus|come here|wake(?: up)? draven)(?: draven| the harness)?$"), lambda m: [("focus", None)]),
    (_TABS, lambda m: [("new_tab", {"n": _count(m.group("n")), "agent": m.group("agent") or "claude"})]),
    (re.compile(r"^(?:show|open|view)(?: me)?(?: the)? plan$"), lambda m: [("open_plan", None)]),
    (re.compile(r"^close(?: this| the)? tab$"), lambda m: [("close_tab", None)]),
    (re.compile(r"^(?:next|forward) tab$"), lambda m: [("select_tab", "next")]),
    (re.compile(r"^(?:previous|prev|last|back) tab$"), lambda m: [("select_tab", "prev")]),
    (re.compile(r"^(?:open )?settings$"), lambda m: [("settings", None)]),
    (re.compile(r"^(?:hide|go away|dismiss)(?: draven)?$"), lambda m: [("hide", None)]),
    (re.compile(r"^(?:stop|pause|quit) listening$"), lambda m: [("pause", None)]),
    (re.compile(r"^(?:start|resume|keep) listening$|^listen$|^wake up$"), lambda m: [("listen", None)]),
    (re.compile(r"^(?:send|go|enter|submit|run it|do it)$"), lambda m: [("enter", None)]),
    (re.compile(r"^(?:tell|ask) (?:claude|draven|codex|hermes)(?: to)? (?P<t>.+)$"), lambda m: [("dictate", m.group("t"))]),
    (re.compile(r"^(?:type|dictate|write|say) (?P<t>.+)$"), lambda m: [("dictate", m.group("t"))]),
]


def parse(text):
    t = normalize(text)
    if not t:
        return []
    for rx, make in _PATTERNS:
        m = rx.match(t)
        if m:
            return make(m)
    # "open three tabs and show the plan" — split on ' and ' / ' then '
    parts = re.split(r"\s+(?:and|then)\s+", t)
    if len(parts) > 1:
        out = []
        for p in parts:
            a = parse(p)
            if not a or a[0][0] in ("unknown", "dictate"):
                out = []
                break
            out.extend(a)
        if out:
            return out
    if len(t.split()) >= 4:
        return [("dictate", t)]
    return [("unknown", t)]
