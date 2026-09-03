#!/usr/bin/env python3
"""
llama-voicechat --serve <-> OpenClaw, with a hands-free web UI.

Two things worth knowing:
  * --system-file is IGNORED in --serve mode (main() only calls run_system() on
    the one-shot path), so the prompt -- and therefore the tool list -- has to go
    over the protocol as {"cmd":"system"}. Miss that and the model politely tells
    the user it cannot do anything, which is true: it has no tools.
  * --serve is turn-based (wav in, wav out). Continuous conversation is done here
    with client-side VAD auto-submitting turns, not by unmuting barge-in, which
    the fork's docs say degenerates every later turn on the timeline.
"""
import argparse, asyncio, json, os, subprocess, sys, threading, time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from urllib.parse import quote
from aiohttp import web

HOME = Path.home()
HERE = Path(__file__).resolve().parent          # projects/open-voice-agent/voicechat
PROJ = HERE.parent
# engine fork and model weights stay outside the project, alongside the other
# llama.cpp checkouts and in ~/models, matching how this box is already laid out
VC   = Path(os.environ.get("VC_BIN",  HOME / "llama-voicechat.cpp/build/bin/llama-voicechat"))
M    = Path(os.environ.get("VC_MODELS", HOME / "models/voicechat/llamacpp"))
SYS  = HERE / "system.txt"
WORK = HERE / "turns"; WORK.mkdir(parents=True, exist_ok=True)
AGENT = os.environ.get("VC_OPENCLAW_AGENT", "main")
TOOL_ACK = os.environ.get(
    "VC_TOOL_ACK",
    "The task is running in the background. Briefly tell the user you started it and will report back.",
)


def model_paths(quant: str):
    # The auxiliary head controls tool/turn decisions, not generated speech.
    # Q4 is currently more reliable than the converted Q8/F16 heads and keeps
    # precision comparisons focused on the actual voice model.
    function_quant = os.environ.get(
        "VC_FUNCTION_QUANT", "Q4_0" if quant in ("Q8_0", "F16") else quant
    )
    return {
        "model": M/f"nemotron_voicechat_11b-stt-llm-{quant}.gguf",
        "mmproj": M/f"mmproj-voicechat-perception-{quant}.gguf",
        "tts": M/f"voicechat-tts-{quant}.gguf",
        "function_head": M/f"nemotron_voicechat_11b-stt-llm-{function_quant}-function-head.gguf",
    }

def log(*a): print(*a, file=sys.stderr, flush=True)


def call_openclaw(request: str) -> str:
    prompt = ("Answer in ONE short spoken sentence. No markdown, no lists, "
              "no preamble. Request: " + request)
    try:
        p = subprocess.run(["openclaw", "agent", "--agent", AGENT, "-m", prompt, "--json"],
                           capture_output=True, text=True, timeout=120)
    except subprocess.TimeoutExpired:
        return "the assistant timed out"
    out = "\n".join(l for l in p.stdout.splitlines() if not l.startswith("["))
    try:
        d = json.loads(out)
    except Exception:
        return "the assistant gave an unreadable answer"
    if d.get("status") != "ok":
        return "the assistant errored: " + str((d.get("error") or {}).get("message", "unknown"))[:120]
    pl = (d.get("result") or {}).get("payloads") or []
    return " ".join(((pl[0].get("text") if pl else "") or "").split())[:220]


class Session:
    def __init__(self, quant: str):
        self.quant = quant
        self.paths = model_paths(quant)
        self.lock = asyncio.Lock()
        self.send_lock = threading.Lock()
        self.ready = threading.Event()
        self.events = []
        self.say_events = []
        self.tool_log = []
        self.loop = asyncio.get_running_loop()
        self.completed = asyncio.Queue()
        self.executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="openclaw")
        self.handoffs = {}

    def start(self):
        missing = [str(p) for p in self.paths.values() if not p.exists()]
        if missing:
            raise FileNotFoundError("missing %s assets:\n%s" % (self.quant, "\n".join(missing)))
        cmd = [str(VC), "-m", str(self.paths["model"]),
               "--mmproj", str(self.paths["mmproj"]),
               "--tts", str(self.paths["tts"]),
               "--function-head", str(self.paths["function_head"]),
               "-ngl", "99", "--device", "Vulkan0", "--serve"]
        env = dict(os.environ, GGML_VK_VISIBLE_DEVICES="0", VC_NO_BARGE="1", VC_FORCE_BOS="1")
        self.p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True, bufsize=1, env=env)
        threading.Thread(target=self._pump, daemon=True).start()

    def _send(self, o):
        with self.send_lock:
            self.p.stdin.write(json.dumps(o) + "\n"); self.p.stdin.flush()

    @staticmethod
    def _parse_tool_call(call):
        req = call
        try:
            arr = json.loads(call[call.index("["):call.rindex("]")+1])
            args = arr[0].get("arguments")
            if isinstance(args, str):
                try: args = json.loads(args)
                except Exception: pass
            req = args.get("request") if isinstance(args, dict) else str(args)
        except Exception as e:
            log(f"[vc] unparsed call ({e}), sending raw")
        return req

    def _run_tool(self, job_id, req, source):
        t = time.time()
        ans = call_openclaw(req)
        dt = time.time() - t
        log(f"[vc] openclaw {job_id} {dt:.1f}s -> {ans[:160]}")
        result = {"job": job_id, "request": req, "answer": ans,
                  "seconds": round(dt, 1), "source": source}
        self.loop.call_soon_threadsafe(self.completed.put_nowait, result)

    def submit_tool(self, req, source="function_head"):
        job_id = f"job-{time.time_ns()}"
        item = {"job": job_id, "request": req, "status": "running", "source": source}
        self.executor.submit(self._run_tool, job_id, req, source)
        return item

    def _pump(self):
        for line in self.p.stdout:
            line = line.strip()
            if not line: continue
            try: ev = json.loads(line)
            except Exception: continue
            k = ev.get("kind")
            if k not in ("assistant_text_delta", "progress"):
                now = time.time()
                d = now - getattr(self, "_last_ev", now)
                self._last_ev = now
                log(f"[ev] +{d:6.2f}s  t={ev.get('t','-'):>5}  {k}")
            if k == "ready":
                log(f"[vc] ready (function_head={ev.get('function_head')}) -> sending system prompt")
                self._send({"cmd": "system", "text": SYS.read_text()})
            elif k == "system_start":
                n = ev.get("tokens", 0)
                log(f"[vc] system prompt: {n} tokens, ~{n*0.08:.0f}s of decode")
            elif k == "system":
                log("[vc] system prompt applied -- tools are live"); self.ready.set()
            elif k == "tool_call_start":
                log("[vc] function head fired (sotc)")
            elif k == "tool_call":
                call = ev.get("text", "")
                log(f"[vc] tool_call: {call!r}")
                req = self._parse_tool_call(call)
                self.tool_log.append(self.submit_tool(req))
                self._send({"cmd": "tool_response", "text": TOOL_ACK})
            elif k == "say_end":
                self.say_events.append(ev)
            elif k in ("turn_end", "error"):
                self.events.append(ev)
                if k == "error": self.say_events.append(ev)

    async def turn(self, wav_in, wav_out):
        async with self.lock:
            self.events = []; self.tool_log = []
            self._last_ev = time.time()
            log(f"[ev] ---- turn start ----")
            self._send({"cmd": "turn", "audio": str(wav_in), "out": str(wav_out)})
            for _ in range(3000):
                await asyncio.sleep(0.1)
                for ev in list(self.events):
                    if ev.get("kind") == "turn_end":
                        text = (ev.get("text") or "").replace("</s>", "").replace("<s>", "").strip()
                        return (text,
                                [dict(item) for item in self.tool_log])
                    if ev.get("kind") == "error":
                        raise RuntimeError(ev.get("message") or ev.get("text", "error"))
            raise TimeoutError("turn timed out")

    async def say(self, text, wav_out):
        async with self.lock:
            self.say_events = []
            self._send({"cmd": "say", "text": text, "out": str(wav_out)})
            for _ in range(3000):
                await asyncio.sleep(0.1)
                for ev in list(self.say_events):
                    if ev.get("kind") == "say_end": return
                    if ev.get("kind") == "error":
                        raise RuntimeError(ev.get("message") or ev.get("text", "error"))
            raise TimeoutError("handoff speech timed out")


PAGE = r"""<!doctype html><html><head><meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<title>VoiceChat + OpenClaw</title><style>
*{box-sizing:border-box} body{font-family:system-ui;max-width:720px;margin:0 auto;padding:1rem;
background:#0e0f13;color:#e8e8ea}
h2{margin:.2rem 0 1rem} .row{display:flex;align-items:center;gap:.75rem;margin-bottom:.75rem}
#pill{padding:.45rem .9rem;border-radius:999px;font-weight:600;font-size:.9rem;background:#333;
transition:background .2s} .idle{background:#3a3f4b!important}
.listen{background:#1f7a3d!important} .speech{background:#2d6cdf!important}
.think{background:#b8860b!important} .talk{background:#7a3dbf!important} .err{background:#a33!important}
#meter{flex:1;height:12px;background:#22242c;border-radius:6px;overflow:hidden}
#bar{height:100%;width:0;background:linear-gradient(90deg,#1f7a3d,#4ade80);transition:width .06s}
#out{height:12px;flex:1;background:#22242c;border-radius:6px;overflow:hidden}
#obar{height:100%;width:0;background:linear-gradient(90deg,#2d6cdf,#93c5fd);transition:width .06s}
button{font-size:1rem;padding:.6rem 1.2rem;border-radius:10px;border:0;background:#2d6cdf;color:#fff;cursor:pointer}
#t{margin-top:1rem;display:flex;flex-direction:column;gap:.6rem}
.m{padding:.6rem .8rem;border-radius:10px;max-width:88%;white-space:pre-wrap;line-height:1.35}
.you{background:#1d2330;align-self:flex-end} .bot{background:#20262e;align-self:flex-start}
.tool{background:#2a2418;align-self:center;font-size:.82rem;color:#e8c988;font-family:ui-monospace,monospace;max-width:100%}
.who{font-size:.7rem;opacity:.55;display:block;margin-bottom:.2rem;text-transform:uppercase;letter-spacing:.06em}
</style></head><body>
<h2>VoiceChat 11B <small style="font-size:.55em;opacity:.55">__QUANT__</small> + OpenClaw</h2>
<div class=row><span id=pill class=idle>idle</span><button id=go>Start conversation</button></div>
<div class=row><span style="font-size:.7rem;opacity:.6;width:2.6rem">in</span><div id=meter><div id=bar></div></div></div>
<div class=row><span style="font-size:.7rem;opacity:.6;width:2.6rem">out</span><div id=out><div id=obar></div></div></div>
<div id=t></div>
<script>
const pill=document.getElementById('pill'),bar=document.getElementById('bar'),
obar=document.getElementById('obar'),T=document.getElementById('t'),go=document.getElementById('go');
let ac,node,stream,on=false,speaking=false,buf=[],pre=[],sil=0,spoke=0,busy=false,handoffs=[];
const TH=0.012, HOLD=0.7, MINSPK=0.35, PRE=6;   // rms gate, end-silence s, min speech s
function setP(c,t){pill.className=c;pill.textContent=t;}
function add(cls,who,txt){const d=document.createElement('div');d.className='m '+cls;
 d.innerHTML='<span class=who></span>';d.firstChild.textContent=who;
 d.appendChild(document.createTextNode(txt));T.appendChild(d);
 window.scrollTo(0,document.body.scrollHeight);return d;}
function wav(fs,sr){let n=fs.reduce((a,c)=>a+c.length,0),b=new ArrayBuffer(44+n*2),v=new DataView(b),o=0;
 const s=t=>{for(let i=0;i<t.length;i++)v.setUint8(o++,t.charCodeAt(i))};
 s("RIFF");v.setUint32(o,36+n*2,true);o+=4;s("WAVEfmt ");v.setUint32(o,16,true);o+=4;
 v.setUint16(o,1,true);o+=2;v.setUint16(o,1,true);o+=2;v.setUint32(o,sr,true);o+=4;
 v.setUint32(o,sr*2,true);o+=4;v.setUint16(o,2,true);o+=2;v.setUint16(o,16,true);o+=2;
 s("data");v.setUint32(o,n*2,true);o+=4;
 for(const f of fs)for(let i=0;i<f.length;i++){let x=Math.max(-1,Math.min(1,f[i]));v.setInt16(o,x*32767,true);o+=2;}
 return new Blob([b],{type:"audio/wav"});}
function resetCapture(){buf=[];pre=[];sil=0;spoke=0;}
function play(blob){return new Promise((resolve,reject)=>{
  const url=URL.createObjectURL(blob), a=new Audio(url);
  setP('talk','speaking');
  const actx=new (window.AudioContext||window.webkitAudioContext)();
  const src=actx.createMediaElementSource(a), an=actx.createAnalyser(); an.fftSize=256;
  src.connect(an); an.connect(actx.destination); const dd=new Uint8Array(an.frequencyBinCount);
  const tick=()=>{ if(a.paused||a.ended){obar.style.width='0%';return;}
    an.getByteTimeDomainData(dd); let m=0; for(const v of dd) m=Math.max(m,Math.abs(v-128)/128);
    obar.style.width=Math.min(100,m*260)+'%'; requestAnimationFrame(tick); };
  a.onplay=tick;
  a.onended=()=>{obar.style.width='0%';actx.close();URL.revokeObjectURL(url);resolve();};
  a.play().catch(e=>{actx.close();URL.revokeObjectURL(url);reject(e);});
});}
async function drainHandoffs(){
  if(busy||!handoffs.length)return;
  busy=true; const h=handoffs.shift(); setP('think','handoff ready');
  try{const r=await fetch(h.audio);if(!r.ok)throw new Error(await r.text());await play(await r.blob());}
  catch(e){add('tool','handoff error',String(e));}
  busy=false;resetCapture();if(on)setP('listen','listening');drainHandoffs();
}
const events=new EventSource('/events');
events.onmessage=e=>{
  const h=JSON.parse(e.data);
  if(h.kind==='handoff_error'){add('tool','handoff error',h.error);return;}
  if(h.kind!=='handoff')return;
  add('tool','openclaw · '+h.seconds+'s',h.request+'\n-> '+h.answer);
  add('bot','voicechat handoff',h.answer); handoffs.push(h); drainHandoffs();
};
events.onerror=()=>console.warn('Background result stream reconnecting');
async function send(fs){
  busy=true; setP('think','thinking');
  const sr=ac.sampleRate, blob=wav(fs,sr), t0=Date.now();
  let r; try{ r=await fetch('/turn',{method:'POST',body:blob}); }
  catch(e){ setP('err','network error'); busy=false; return; }
  if(!r.ok){ add('tool','error',await r.text()); setP('err','error'); busy=false; setTimeout(()=>{if(on)setP('listen','listening')},1500); return; }
  const meta=JSON.parse(decodeURIComponent(r.headers.get('X-Meta')||'{}'));
  if(meta.user) add('you','you',meta.user);
  (meta.tools||[]).forEach(x=>add('tool','openclaw · running · '+x.source,x.request));
  add('bot','voicechat '+'__QUANT__'+' ('+((Date.now()-t0)/1000).toFixed(1)+'s)',meta.text||'(no text)');
  try{await play(await r.blob());}catch(e){add('tool','audio error',String(e));}
  busy=false;resetCapture();if(on)setP('listen','listening');drainHandoffs();
}
async function start(){
  stream=await navigator.mediaDevices.getUserMedia({audio:{channelCount:1,echoCancellation:true,
    noiseSuppression:true,autoGainControl:true}});
  ac=new AudioContext({sampleRate:16000});
  const src=ac.createMediaStreamSource(stream);
  node=ac.createScriptProcessor(2048,1,1); src.connect(node); node.connect(ac.destination);
  const dt=2048/ac.sampleRate;
  node.onaudioprocess=e=>{
    const d=e.inputBuffer.getChannelData(0); let s=0; for(let i=0;i<d.length;i++)s+=d[i]*d[i];
    const rms=Math.sqrt(s/d.length); bar.style.width=Math.min(100,rms*1400)+'%';
    if(!on||busy) return;
    const loud=rms>TH;
    if(!speaking){
      pre.push(new Float32Array(d)); if(pre.length>PRE) pre.shift();
      if(loud){ speaking=true; buf=pre.slice(); pre=[]; spoke=0; sil=0; setP('speech','you are speaking'); }
    } else {
      buf.push(new Float32Array(d)); spoke+=dt;
      sil = loud ? 0 : sil+dt;
      if(sil>=HOLD){ speaking=false;
        if(spoke-sil>=MINSPK){ const f=buf; buf=[]; send(f); } else { buf=[]; setP('listen','listening'); } }
    }
  };
  on=true; go.textContent='Stop'; setP('listen','listening');
  add('tool','ready','Mic open at '+ac.sampleRate+' Hz. Just talk - it sends automatically when you stop.');
}
go.onclick=async()=>{ if(on){ on=false; stream.getTracks().forEach(t=>t.stop()); ac.close();
    go.textContent='Start conversation'; setP('idle','idle'); } else { try{await start()}catch(e){setP('err','mic denied')} } };
</script></body></html>"""


async def main(args):
    sess = Session(args.quant); sess.start()
    log(f"[vc] loading {args.quant} model...")
    while not sess.ready.is_set():
        await asyncio.sleep(0.2)
    log("[vc] session ready")

    asr = {"m": None}
    def transcribe(path):
        try:
            if asr["m"] is None:
                import whisper
                asr["m"] = whisper.load_model("small.en", device="cuda")
                log("[vc] whisper loaded for user transcript")
            r = asr["m"].transcribe(str(path), language="en", fp16=False,
                                    condition_on_previous_text=False)
            return (r.get("text") or "").strip()
        except Exception as e:
            log(f"[vc] asr failed: {e}"); return ""

    subscribers = set()

    async def publish(item):
        for q in list(subscribers):
            try: q.put_nowait(item)
            except asyncio.QueueFull: log("[vc] dropping event for a slow browser")

    async def dispatch_handoffs():
        while True:
            item = await sess.completed.get()
            fout = WORK/f"handoff-{item['job']}.wav"
            try:
                await sess.say(item["answer"], fout)
                if not fout.exists(): raise RuntimeError("no handoff audio produced")
                sess.handoffs[item["job"]] = fout
                item.update({"kind": "handoff", "audio": f"/handoff/{item['job']}.wav"})
            except Exception as e:
                log(f"[vc] handoff {item['job']} failed: {type(e).__name__}: {e}")
                item.update({"kind": "handoff_error", "error": str(e)})
            await publish(item)
            sess.completed.task_done()

    asyncio.create_task(dispatch_handoffs())

    async def index(_):
        return web.Response(text=PAGE.replace("__QUANT__", sess.quant), content_type="text/html")

    async def events(request):
        response = web.StreamResponse(headers={
            "Content-Type": "text/event-stream",
            "Cache-Control": "no-cache",
            "Connection": "keep-alive",
        })
        await response.prepare(request)
        q = asyncio.Queue(maxsize=32); subscribers.add(q)
        try:
            await response.write(b": connected\n\n")
            while True:
                try: item = await asyncio.wait_for(q.get(), timeout=15)
                except asyncio.TimeoutError:
                    await response.write(b": keepalive\n\n"); continue
                payload = json.dumps(item, separators=(",", ":"))
                await response.write(f"id: {item.get('job', '')}\ndata: {payload}\n\n".encode())
        except (ConnectionResetError, asyncio.CancelledError):
            pass
        finally:
            subscribers.discard(q)
        return response

    async def handoff_audio(request):
        job = request.match_info["job"]
        path = sess.handoffs.get(job)
        if path is None or not path.exists(): raise web.HTTPNotFound()
        return web.FileResponse(path, headers={"Cache-Control": "no-store"})

    async def turn(request):
        data = await request.read()
        n = int(time.time()*1000)
        fin, fout = WORK/f"in-{n}.wav", WORK/f"out-{n}.wav"
        fin.write_bytes(data)
        loop = asyncio.get_running_loop()
        asr_task = loop.run_in_executor(None, transcribe, fin)   # parallel, adds no latency
        try:
            text, tools = await sess.turn(fin, fout)
        except Exception as e:
            return web.Response(status=500, text=f"{type(e).__name__}: {e}")
        user = await asr_task
        if not tools and "openclaw" in user.lower():
            fallback = sess.submit_tool(user, source="explicit_name_fallback")
            tools.append(fallback)
            log(f"[vc] {fallback['job']} queued by explicit-name fallback")
        if not fout.exists():
            return web.Response(status=500, text="no audio produced")
        meta = {"text": text, "user": user, "tools": tools}
        return web.Response(body=fout.read_bytes(), content_type="audio/wav",
                            headers={"X-Meta": quote(json.dumps(meta))})

    app = web.Application(client_max_size=64*1024*1024)
    app.add_routes([web.get("/", index), web.get("/events", events),
                    web.get("/handoff/{job}.wav", handoff_audio), web.post("/turn", turn)])
    import ssl
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(PROJ/"personaplex/ssl/cert.pem", PROJ/"personaplex/ssl/key.pem")
    log(f"[vc] {args.quant} UI on https://192.168.68.46:{args.port}")
    await web._run_app(app, port=args.port, ssl_context=ctx, print=None)

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="VoiceChat 11B + async OpenClaw web UI")
    parser.add_argument("--quant", choices=("Q4_0", "Q8_0", "F16"),
                        default=os.environ.get("VC_QUANT", "Q4_0").upper())
    parser.add_argument("--port", type=int, default=int(os.environ.get("VC_PORT", "8999")))
    asyncio.run(main(parser.parse_args()))
