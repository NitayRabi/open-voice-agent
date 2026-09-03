import { FC, useRef } from "react";
import { AudioStats, useServerAudio } from "../../hooks/useServerAudio";
import { ServerVisualizer } from "../AudioVisualizer/ServerVisualizer";
import { type ThemeType } from "../../hooks/useSystemTheme";

type ServerAudioProps = {
  setGetAudioStats: (getAudioStats: () => AudioStats) => void;
  theme: ThemeType;
};
export const ServerAudio: FC<ServerAudioProps> = ({ setGetAudioStats, theme }) => {
  const { analyser, hasCriticalDelay, setHasCriticalDelay } = useServerAudio({
    setGetAudioStats,
  });
  const containerRef = useRef<HTMLDivElement>(null);
  return (
    <>
      {hasCriticalDelay && (
        <div className="fixed left-0 top-0 flex w-screen justify-between bg-red-500 p-2 text-center">
          <p>A connection issue has been detected, you've been reconnected</p>
          <button
            onClick={async () => {
              setHasCriticalDelay(false);
            }}
            className="bg-white p-1 text-black"
          >
            Dismiss
          </button>
        </div>
      )}
      <div className="flex flex-col items-center gap-3">
        <div className="relative flex h-32 w-32 items-center justify-center rounded-full border border-white/15 bg-white/5">
          <div className="absolute h-24 w-24 animate-pulse rounded-full bg-[#58d69b]/10" />
          <div className="absolute h-14 w-14 rounded-full bg-gradient-to-br from-[#8fe0ba] to-[#36a878] shadow-[0_0_45px_rgba(88,214,155,0.35)]" />
          <div className="absolute h-3 w-3 rounded-full bg-white/80" />
          <div className="pointer-events-none absolute inset-0 opacity-0" ref={containerRef}>
            <ServerVisualizer analyser={analyser.current} parent={containerRef} theme={theme}/>
          </div>
        </div>
        <span className="text-xs font-medium text-white/55">PersonaPlex</span>
      </div>
    </>
  );
};
