"""
Delegation bridge: PersonaPlex (voice) -> tool router -> OpenClaw agent.

Mirrors the pattern Codex CLI uses for realtime voice: the speech model carries
exactly ONE tool, which hands the whole task to a real agent. PersonaPlex has no
function calling, so a local Gemma model reads the user/assistant transcript and
decides whether to call ``delegate(message)``.

Everything expensive (ASR, the agent turn) runs on a worker thread. The audio
loop only ever calls feed()/poll(), which are O(1) and never block -- the frame
budget is 80ms and we are not spending it here.
"""
import json
import os
import queue
import subprocess
import threading
import time
import urllib.request
from difflib import SequenceMatcher
import re

import numpy as np

class OpenClawBridge:
    def __init__(self, sample_rate=24000, agent="main", asr_model="small.en",
                 device="cuda", log=print, timeout=90,
                 router_url="http://127.0.0.1:8011/v1/chat/completions",
                 router_model="gemma-e4b"):
        self.sr = sample_rate
        self.agent = agent
        self.log = log
        self.timeout = timeout
        self.router_url = router_url
        self.router_model = router_model
        self._buf = []              # pending user audio (float32 @ sr)
        self._lock = threading.Lock()
        self._assistant_text = ""
        self._out = queue.Queue()   # text ready to be spoken
        self._events = queue.Queue()  # structured trace events for the Web UI
        self._jobs = queue.Queue()
        self._busy = False
        self._response_gate = False
        self._last_utterance = ("", 0.0)
        self._delegation_lock = threading.Lock()
        self._active_delegation = None
        self._recent_delegations = []
        self._next_delegation_id = 1

        import whisper
        t = time.time()
        self.asr = whisper.load_model(asr_model, device=device)
        self.log(f"[bridge] whisper '{asr_model}' loaded in {time.time()-t:.1f}s on {device}")

        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._worker, daemon=True)
        self._thread.start()
        self._tool_thread = threading.Thread(target=self._tool_worker, daemon=True)
        self._tool_thread.start()

    # ---- audio-loop side: must stay cheap -------------------------------
    def feed(self, pcm):
        with self._lock:
            self._buf.append(pcm.copy())

    def feed_assistant_text(self, piece):
        """Receive PersonaPlex's inner text stream without blocking audio."""
        with self._lock:
            self._assistant_text = (self._assistant_text + piece)[-2000:]

    def poll(self):
        try:
            text = self._out.get_nowait()
            # The audio loop injects this text immediately, so spontaneous model
            # output can safely resume behind the forced result.
            self._response_gate = False
            return text
        except queue.Empty:
            return None

    def poll_event(self):
        try:
            return self._events.get_nowait()
        except queue.Empty:
            return None

    def _emit(self, event_type, **data):
        self._events.put({"type": event_type, "at": time.time(), **data})

    @property
    def busy(self):
        return self._busy

    @property
    def response_gate(self):
        return self._response_gate

    def close(self):
        self._stop.set()

    # ---- worker thread --------------------------------------------------
    def _drain(self):
        with self._lock:
            if not self._buf:
                return None
            a = np.concatenate(self._buf)
            self._buf = []
            return a

    def _worker(self):
        pending = np.zeros(0, dtype=np.float32)
        silence_for = 0.0
        SIL_RMS = 0.006          # below this counts as silence
        SIL_HOLD = 0.8           # seconds of silence that ends an utterance
        MAX_UTT = 15.0

        while not self._stop.is_set():
            time.sleep(0.15)
            chunk = self._drain()
            if chunk is None or len(chunk) == 0:
                continue
            pending = np.concatenate([pending, chunk])
            rms = float(np.sqrt(np.mean(chunk.astype(np.float64) ** 2)))
            dur = len(chunk) / self.sr
            silence_for = silence_for + dur if rms < SIL_RMS else 0.0
            if rms >= SIL_RMS and len(pending) / self.sr >= 0.2:
                # Gate PersonaPlex as soon as user speech is established. It still
                # consumes audio frames, but cannot race the router with an answer.
                self._response_gate = True

            long_enough = len(pending) / self.sr > 0.6
            ended = silence_for >= SIL_HOLD and long_enough
            too_long = len(pending) / self.sr >= MAX_UTT
            if not (ended or too_long):
                continue

            utt, pending, silence_for = pending, np.zeros(0, dtype=np.float32), 0.0
            if float(np.sqrt(np.mean(utt.astype(np.float64) ** 2))) < SIL_RMS:
                self._response_gate = False
                continue
            try:
                self._handle(utt)
            except Exception as e:
                self.log(f"[bridge] error: {type(e).__name__}: {e}")
                self._emit("error", message=f"{type(e).__name__}: {e}")
                self._response_gate = False

    def _handle(self, utt):
        import whisper as _w
        # whisper wants 16k mono float32
        n16 = int(len(utt) * 16000 / self.sr)
        a16 = np.interp(np.linspace(0, len(utt) - 1, n16),
                        np.arange(len(utt)), utt).astype(np.float32)
        t = time.time()
        r = self.asr.transcribe(a16, language="en", fp16=False,
                                condition_on_previous_text=False)
        text = (r.get("text") or "").strip()
        if not text:
            self._response_gate = False
            return
        asr_seconds = time.time() - t
        self.log(f"[bridge] heard ({asr_seconds:.1f}s): {text!r}")
        self._emit("user_transcript", text=text, seconds=round(asr_seconds, 3))

        fingerprint = " ".join(text.lower().split())
        if fingerprint == self._last_utterance[0] and time.time() - self._last_utterance[1] < 8:
            self.log("[bridge] ignored duplicate utterance")
            self._response_gate = bool(self._active_delegation)
            return
        self._last_utterance = (fingerprint, time.time())
        with self._lock:
            assistant_context = self._assistant_text.strip()
            self._assistant_text = ""

        # Short correction fragments are not standalone actions. Waiting here is
        # safer than turning "No, I mean..." into a second speculative call.
        words = [w for w in fingerprint.split() if w.strip(".,!?;:")]
        if len(words) <= 4 and words and words[0].strip(".,!?;:") in {
            "no", "and", "but", "wait", "actually", "sorry"
        }:
            self._emit("call_suppressed", reason="incomplete follow-up; waiting for the full request")
            self._response_gate = bool(self._active_delegation)
            return

        self._emit("routing", model=self.router_model)
        t = time.time()
        task = self._route(text, assistant_context)
        router_seconds = time.time() - t
        self.log(f"[bridge] router ({router_seconds:.2f}s): {task!r}")
        if not task:
            self._emit("route_skip", model=self.router_model,
                       seconds=round(router_seconds, 3))
            self._response_gate = False
            return

        duplicate = self._find_duplicate(task)
        explicit_repeat = any(word in fingerprint.split() for word in {
            "again", "repeat", "retry", "refresh", "recheck"
        })
        active_duplicate = duplicate and duplicate.get("status") in {"pending", "running"}
        if duplicate is not None and (active_duplicate or not explicit_repeat):
            self.log(f"[bridge] suppressed duplicate of delegation {duplicate['id']}: {task!r}")
            self._emit("call_suppressed", reason="same intent is already active or recently completed",
                       duplicate_of=duplicate["id"])
            self._response_gate = bool(active_duplicate)
            return

        with self._delegation_lock:
            delegation = {
                "id": self._next_delegation_id,
                "task": task,
                "status": "pending",
                "created_at": time.time(),
            }
            self._next_delegation_id += 1
            self._active_delegation = delegation
            self._busy = True
        self._emit("tool_call", id=delegation["id"], model=self.router_model,
                   tool="delegate", arguments={"message": task},
                   seconds=round(router_seconds, 3), status="pending")
        self._out.put("I'm checking that right now.")
        self._jobs.put(delegation)

    def _tool_worker(self):
        """Execute tools separately so ASR/routing stays live during a slow call."""
        while not self._stop.is_set():
            try:
                delegation = self._jobs.get(timeout=0.2)
            except queue.Empty:
                continue
            delegation["status"] = "running"
            self._emit("tool_status", id=delegation["id"], tool="delegate", status="running")
            self.log(f"[bridge] -> delegate/openclaw id={delegation['id']} agent={self.agent}: {delegation['task']!r}")
            t = time.time()
            try:
                reply = self._call_openclaw(delegation["task"])
                tool_seconds = time.time() - t
                delegation.update(status="completed", completed_at=time.time(), result=reply)
                self.log(f"[bridge] <- delegate/openclaw id={delegation['id']} ({tool_seconds:.1f}s): {reply!r}")
                self._emit("tool_result", id=delegation["id"], tool="delegate",
                           executor="OpenClaw", text=reply,
                           seconds=round(tool_seconds, 3), status="completed")
                if reply:
                    self._out.put(self._for_speech(reply))
            except Exception as exc:
                delegation.update(status="failed", completed_at=time.time())
                self._emit("error", id=delegation["id"],
                           message=f"{type(exc).__name__}: {exc}")
                self._out.put("I couldn't complete that request.")
            finally:
                with self._delegation_lock:
                    self._recent_delegations.append(delegation.copy())
                    self._recent_delegations = self._recent_delegations[-8:]
                    if (self._active_delegation or {}).get("id") == delegation["id"]:
                        self._active_delegation = None
                    self._busy = False

    @staticmethod
    def _intent_key(task):
        return " ".join(
            word.strip(".,!?;:()[]{}\"'").lower()
            for word in task.split()
            if word.strip(".,!?;:()[]{}\"'").lower() not in {
                "a", "an", "the", "to", "for", "please", "can", "you", "my", "me"
            }
        )

    def _find_duplicate(self, task):
        key = self._intent_key(task)
        now = time.time()
        with self._delegation_lock:
            candidates = ([self._active_delegation] if self._active_delegation else []) + [
                d for d in self._recent_delegations
                if now - d.get("completed_at", d["created_at"]) < 45
            ]
        for previous in candidates:
            old_key = self._intent_key(previous["task"])
            if key == old_key or SequenceMatcher(None, key, old_key).ratio() >= 0.72:
                return previous
        return None

    @staticmethod
    def _for_speech(text):
        """Keep a multi-sentence result inside one PersonaPlex speaking turn."""
        parts = [
            part.strip().rstrip(".!?")
            for part in re.split(r"(?<=[.!?])\s+", text)
            if part.strip()
        ]
        return ("; ".join(parts) + ".") if parts else text

    def _route(self, user_text, assistant_context):
        """Return the delegate() argument, or None when voice should handle it."""
        system = (
            "You are the tool router for a live voice conversation. You have one "
            "tool, delegate, which sends a message to a capable external agent. "
            "Call it when the user asks to retrieve current/private information, "
            "use files/devices/apps, or perform an action. Also call it when the "
            "user explicitly asks to delegate, ask the agent, or ask OpenClaw. "
            "Do not call it for greetings, casual conversation, opinions, jokes, "
            "or questions answerable from general knowledge. Never answer the user. "
            "Never repeat an active or recently completed delegation with the same "
            "intent. A correction fragment is not a new call; wait for a complete "
            "standalone request."
        )
        with self._delegation_lock:
            active = dict(self._active_delegation) if self._active_delegation else None
            recent = [
                {"id": d["id"], "task": d["task"], "status": d["status"]}
                for d in self._recent_delegations[-4:]
            ]
        context = assistant_context or "(no recent assistant transcript)"
        body = {
            "model": self.router_model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": (
                    "Recent voice-model transcript:\n" + context +
                    "\n\nDelegation state:\n" + json.dumps({"active": active, "recent": recent}) +
                    "\n\nLatest user utterance:\n" + user_text
                )},
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "delegate",
                    "description": "Send the user's task to an external agent that can use tools.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "message": {"type": "string", "description": "Complete request for the agent"}
                        },
                        "required": ["message"],
                    },
                },
            }],
            "tool_choice": "auto",
            "temperature": 0,
            "max_tokens": 96,
            "chat_template_kwargs": {"enable_thinking": False},
        }
        req = urllib.request.Request(
            self.router_url,
            data=json.dumps(body).encode("utf-8"),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=10) as response:
                result = json.load(response)
        except Exception as exc:
            self.log(f"[bridge] router unavailable: {type(exc).__name__}: {exc}")
            return None
        message = result.get("choices", [{}])[0].get("message", {})
        calls = message.get("tool_calls") or []
        for call in calls:
            function = call.get("function") or {}
            if function.get("name") != "delegate":
                continue
            try:
                args = json.loads(function.get("arguments") or "{}")
            except json.JSONDecodeError:
                continue
            task = str(args.get("message") or "").strip()
            if task:
                return task[:2000]
        return None

    def _call_openclaw(self, task):
        prompt = (
            "Answer in exactly one complete spoken sentence of at most 40 words. "
            "Include every important result, use plain English, and do not use "
            "markdown, lists, or code blocks. Question: " + task
        )
        try:
            p = subprocess.run(
                ["openclaw", "agent", "--agent", self.agent, "-m", prompt, "--json"],
                capture_output=True, text=True, timeout=self.timeout,
            )
        except subprocess.TimeoutExpired:
            return "My assistant took too long to answer."
        out = "\n".join(l for l in p.stdout.splitlines() if not l.startswith("["))
        try:
            d = json.loads(out)
        except Exception:
            return "My assistant returned something I could not read."
        if d.get("status") != "ok":
            err = (d.get("error") or {}).get("message", "unknown error")
            return f"My assistant reported an error: {err[:160]}"
        pl = (d.get("result") or {}).get("payloads") or []
        txt = (pl[0].get("text") if pl else "") or ""
        txt = " ".join(txt.split())
        return txt[:600] or "My assistant sent an empty reply."
