import moshiProcessorUrl from "../../audio-processor.ts?worker&url";
import { FC, useEffect, useState, useCallback, useRef, MutableRefObject } from "react";
import eruda from "eruda";
import { useSearchParams } from "react-router-dom";
import { Conversation } from "../Conversation/Conversation";
import { Button } from "../../components/Button/Button";
import { useModelParams } from "../Conversation/hooks/useModelParams";
import { env } from "../../env";
import { prewarmDecoderWorker } from "../../decoder/decoderWorker";

const VOICE_OPTIONS = [
  "NATF0.pt", "NATF1.pt", "NATF2.pt", "NATF3.pt",
  "NATM0.pt", "NATM1.pt", "NATM2.pt", "NATM3.pt",
  "VARF0.pt", "VARF1.pt", "VARF2.pt", "VARF3.pt", "VARF4.pt",
  "VARM0.pt", "VARM1.pt", "VARM2.pt", "VARM3.pt", "VARM4.pt",
];

interface HomepageProps {
  showMicrophoneAccessMessage: boolean;
  startConnection: () => Promise<void>;
  voicePrompt: string;
  setVoicePrompt: (value: string) => void;
  voiceModel: string;
  setVoiceModel: (value: string) => void;
  toolModel: string;
  setToolModel: (value: string) => void;
}

const Homepage = ({
  startConnection,
  showMicrophoneAccessMessage,
  voicePrompt,
  setVoicePrompt,
  voiceModel,
  setVoiceModel,
  toolModel,
  setToolModel,
}: HomepageProps) => {
  return (
    <div className="min-h-screen w-full bg-[#f5f3ef] px-5 py-8 text-[#17201d] md:px-10 md:py-12">
      <div className="mx-auto max-w-5xl">
        <header className="mb-10 flex items-center gap-3">
          <div className="flex h-11 w-11 items-center justify-center rounded-2xl bg-[#17201d] text-lg font-semibold text-white">O</div>
          <div>
            <h1 className="text-2xl font-semibold tracking-tight">Open Voice Lab</h1>
            <p className="text-sm text-[#66716d]">Duplex conversation with observable tool delegation</p>
          </div>
        </header>

        <main className="grid gap-6 lg:grid-cols-[1.2fr_0.8fr]">
          <section className="rounded-[28px] border border-black/10 bg-white p-7 shadow-[0_24px_70px_rgba(31,41,37,0.08)] md:p-10">
            <div className="mb-8">
              <span className="rounded-full bg-[#e1f4ea] px-3 py-1 text-xs font-semibold text-[#176b48]">LOCAL POC</span>
              <h2 className="mt-5 max-w-xl text-4xl font-medium leading-tight tracking-[-0.04em]">Talk naturally. Tools only step in when they’re needed.</h2>
              <p className="mt-4 max-w-xl text-base leading-7 text-[#66716d]">PersonaPlex handles the live conversation. E4B inspects each turn, delegates external work, and returns one result for the voice to deliver.</p>
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <label className="block rounded-2xl border border-black/10 bg-[#faf9f6] p-4">
                <span className="mb-2 block text-xs font-semibold uppercase tracking-[0.12em] text-[#7a847f]">Voice model</span>
                <select value={voiceModel} onChange={(e) => setVoiceModel(e.target.value)} className="w-full bg-transparent text-sm font-semibold outline-none">
                  <option value="personaplex-7b">PersonaPlex 7B · BF16</option>
                  <option disabled>Nemotron VoiceChat 11B · not connected</option>
                </select>
              </label>
              <label className="block rounded-2xl border border-black/10 bg-[#faf9f6] p-4">
                <span className="mb-2 block text-xs font-semibold uppercase tracking-[0.12em] text-[#7a847f]">Tool router</span>
                <select value={toolModel} onChange={(e) => setToolModel(e.target.value)} className="w-full bg-transparent text-sm font-semibold outline-none">
                  <option value="gemma-e4b-q4">Gemma 4 E4B · Q4_0</option>
                </select>
              </label>
            </div>

            <label className="mt-4 block rounded-2xl border border-black/10 bg-[#faf9f6] p-4">
              <span className="mb-2 block text-xs font-semibold uppercase tracking-[0.12em] text-[#7a847f]">Voice</span>
          <select
            id="voice-prompt"
            name="voice-prompt"
            value={voicePrompt}
            onChange={(e) => setVoicePrompt(e.target.value)}
            className="w-full bg-transparent text-sm font-semibold outline-none"
          >
            {VOICE_OPTIONS.map((voice) => (
              <option key={voice} value={voice}>
                {voice
                  .replace('.pt', '')
                  .replace(/^NAT/, 'NATURAL_')
                  .replace(/^VAR/, 'VARIETY_')}
              </option>
            ))}
          </select>
            </label>

            {showMicrophoneAccessMessage && <p className="mt-4 text-sm text-red-600">Microphone access is required to start.</p>}
            <Button onClick={async () => await startConnection()} className="mt-6 w-full !rounded-2xl !bg-[#17201d] !px-5 !py-4 !font-semibold !text-white hover:!bg-[#2b3732]">Start conversation</Button>
          </section>

          <aside className="rounded-[28px] bg-[#17201d] p-7 text-white md:p-8">
            <p className="text-xs font-semibold uppercase tracking-[0.16em] text-[#8fd6b5]">How it behaves</p>
            <div className="mt-6 space-y-6">
              <div><div className="text-sm font-semibold">Conversation stays fluid</div><p className="mt-1 text-sm leading-6 text-white/60">Greetings, opinions, and general knowledge stay entirely with the voice model.</p></div>
              <div><div className="text-sm font-semibold">External work is gated</div><p className="mt-1 text-sm leading-6 text-white/60">Files, devices, live data, and actions wait for E4B before the voice responds.</p></div>
              <div><div className="text-sm font-semibold">Every decision is visible</div><p className="mt-1 text-sm leading-6 text-white/60">The timeline shows ASR, routing, calls, suppression, latency, and results.</p></div>
            </div>
            <div className="mt-8 rounded-2xl border border-white/10 bg-white/5 p-4">
              <p className="text-xs uppercase tracking-wider text-white/40">Try saying</p>
              <p className="mt-2 text-sm leading-6">“What’s in my Downloads folder?”</p>
            </div>
          </aside>
        </main>
      </div>
    </div>
  );
}

export const Queue:FC = () => {
  const theme = "light" as const;  // Always use light theme
  const [searchParams] = useSearchParams();
  const overrideWorkerAddr = searchParams.get("worker_addr");
  const [hasMicrophoneAccess, setHasMicrophoneAccess] = useState<boolean>(false);
  const [showMicrophoneAccessMessage, setShowMicrophoneAccessMessage] = useState<boolean>(false);
  const modelParams = useModelParams();
  const [voiceModel, setVoiceModel] = useState("personaplex-7b");
  const [toolModel, setToolModel] = useState("gemma-e4b-q4");

  const audioContext = useRef<AudioContext | null>(null);
  const worklet = useRef<AudioWorkletNode | null>(null);
  
  // enable eruda in development
  useEffect(() => {
    if(env.VITE_ENV === "development") {
      eruda.init();
    }
    () => {
      if(env.VITE_ENV === "development") {
        eruda.destroy();
      }
    };
  }, []);

  const getMicrophoneAccess = useCallback(async () => {
    try {
      await window.navigator.mediaDevices.getUserMedia({ audio: true });
      setHasMicrophoneAccess(true);
      return true;
    } catch(e) {
      console.error(e);
      setShowMicrophoneAccessMessage(true);
      setHasMicrophoneAccess(false);
    }
    return false;
}, [setHasMicrophoneAccess, setShowMicrophoneAccessMessage]);

  const startProcessor = useCallback(async () => {
    if(!audioContext.current) {
      audioContext.current = new AudioContext();
      // Prewarm decoder worker as soon as we have audio context
      // This gives WASM time to load while user grants mic access
      prewarmDecoderWorker(audioContext.current.sampleRate);
    }
    if(worklet.current) {
      return;
    }
    let ctx = audioContext.current;
    ctx.resume();
    try {
      worklet.current = new AudioWorkletNode(ctx, 'moshi-processor');
    } catch (err) {
      await ctx.audioWorklet.addModule(moshiProcessorUrl);
      worklet.current = new AudioWorkletNode(ctx, 'moshi-processor');
    }
    worklet.current.connect(ctx.destination);
  }, [audioContext, worklet]);

  const startConnection = useCallback(async() => {
      await startProcessor();
      const hasAccess = await getMicrophoneAccess();
      if (hasAccess) {
      // Values are already set in modelParams, they get passed to Conversation
    }
  }, [startProcessor, getMicrophoneAccess]);

  return (
    <>
      {(hasMicrophoneAccess && audioContext.current && worklet.current) ? (
        <Conversation
        workerAddr={overrideWorkerAddr ?? ""}
        audioContext={audioContext as MutableRefObject<AudioContext|null>}
        worklet={worklet as MutableRefObject<AudioWorkletNode|null>}
        theme={theme}
        startConnection={startConnection}
        voiceModel="PersonaPlex 7B · BF16"
        toolModel="Gemma 4 E4B · Q4_0"
        {...modelParams}
        />
      ) : (
        <Homepage
          startConnection={startConnection}
          showMicrophoneAccessMessage={showMicrophoneAccessMessage}
          voicePrompt={modelParams.voicePrompt}
          setVoicePrompt={modelParams.setVoicePrompt}
          voiceModel={voiceModel}
          setVoiceModel={setVoiceModel}
          toolModel={toolModel}
          setToolModel={setToolModel}
        />
      )}
    </>
  );
};
