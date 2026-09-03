import { FC, useCallback, useEffect, useRef, useState } from "react";
import { useSocketContext } from "../../SocketContext";
import { useUserAudio } from "../../hooks/useUserAudio";
import { ClientVisualizer } from "../AudioVisualizer/ClientVisualizer";
import { type ThemeType } from "../../hooks/useSystemTheme";

type UserAudioProps = {
  theme: ThemeType;
};
export const UserAudio: FC<UserAudioProps> = ({theme}) => {
  const [analyser, setAnalyser] = useState<AnalyserNode | null>(null);
  const { sendMessage, socketStatus } = useSocketContext();
  const containerRef = useRef<HTMLDivElement>(null);
  const onRecordingStart = useCallback(() => {
    console.log("Recording started");
  }, []);

  const onRecordingStop = useCallback(() => {
    console.log("Recording stopped");
  }, []);

  const onRecordingChunk = useCallback(
    (chunk: Uint8Array) => {
      if (socketStatus !== "connected") {
        return;
      }
      sendMessage({
        type: "audio",
        data: chunk,
      });
    },
    [sendMessage, socketStatus],
  );

  const { startRecordingUser, stopRecording } = useUserAudio({
    constraints: {
      audio: {
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
        channelCount: 1,
      },
      video: false,
    },
    onDataChunk: onRecordingChunk,
    onRecordingStart,
    onRecordingStop,
  });

  useEffect(() => {
    let res: Awaited<ReturnType<typeof startRecordingUser>>;
    if (socketStatus === "connected") {
      startRecordingUser().then(result => {
        if (result) {
          res = result;
          setAnalyser(result.analyser);
        }
      });
    }
    return () => {
      console.log("Stop recording called from somewhere else.");
      stopRecording();
      res?.source?.disconnect();
    };
  }, [startRecordingUser, stopRecording, socketStatus]);

  return (
    <div className="flex flex-col items-center gap-3">
      <div className="relative flex h-32 w-32 items-center justify-center rounded-full border border-white/15 bg-white/[0.03]">
        <div className="flex h-12 items-center gap-1">
          {[16, 30, 42, 26, 36, 20].map((height, index) => <span key={index} className="w-1 rounded-full bg-white/70" style={{height}} />)}
        </div>
        <div className="pointer-events-none absolute inset-0 opacity-0" ref={containerRef}>
          <ClientVisualizer theme={theme} analyser={analyser} parent={containerRef}/>
        </div>
      </div>
      <span className="text-xs font-medium text-white/55">You</span>
    </div>
  );
};
