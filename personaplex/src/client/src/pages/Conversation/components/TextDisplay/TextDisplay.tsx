import { FC, useEffect, useRef } from "react";
import { useServerText } from "../../hooks/useServerText";

type TextDisplayProps = {
  containerRef: React.RefObject<HTMLDivElement>;
};

export const TextDisplay:FC<TextDisplayProps> = ({
  containerRef,
}) => {
  const { items } = useServerText();
  const prevScrollTop = useRef(0);

  useEffect(() => {
    if (containerRef.current) {
      prevScrollTop.current = containerRef.current.scrollTop;
      containerRef.current.scroll({
        top: containerRef.current.scrollHeight,
        behavior: "smooth",
      });
    }
  }, [items]);

  const eventCard = (event: Extract<(typeof items)[number], { kind: "toolEvent" }>["data"], i: number) => {
    const timing = event.seconds === undefined ? "" : ` · ${event.seconds.toFixed(2)}s`;
    if (event.type === "user_transcript") {
      return <div key={i} className="my-2 rounded-lg border border-slate-300 bg-slate-50 p-2 text-slate-700">
        <div className="text-xs font-semibold uppercase tracking-wide text-slate-500">You · Whisper{timing}</div>
        <div>{event.text}</div>
      </div>;
    }
    if (event.type === "routing") {
      return <div key={i} className="my-2 text-xs font-semibold text-violet-600">E4B is deciding whether to delegate…</div>;
    }
    if (event.type === "route_skip") {
      return <div key={i} className="my-2 rounded-lg border border-emerald-200 bg-emerald-50 p-2 text-sm text-emerald-800">
        E4B · no tool call{timing}
      </div>;
    }
    if (event.type === "call_suppressed") {
      return <div key={i} className="my-2 rounded-lg border border-sky-200 bg-sky-50 p-2 text-sm text-sky-900">
        E4B · tool call suppressed — {event.reason}
      </div>;
    }
    if (event.type === "tool_call") {
      return <div key={i} className="my-2 rounded-lg border border-violet-300 bg-violet-50 p-2 text-violet-950">
        <div className="text-xs font-semibold uppercase tracking-wide text-violet-600">E4B → {event.tool} #{event.id}{timing}</div>
        <div className="font-mono text-sm">{event.arguments?.message}</div>
      </div>;
    }
    if (event.type === "tool_status") {
      return <div key={i} className="my-2 text-xs font-semibold text-violet-600">
        Tool call #{event.id} · {event.status}
      </div>;
    }
    if (event.type === "tool_result") {
      return <div key={i} className="my-2 rounded-lg border border-amber-300 bg-amber-50 p-2 text-amber-950">
        <div className="text-xs font-semibold uppercase tracking-wide text-amber-700">{event.executor} result{timing}</div>
        <div>{event.text}</div>
      </div>;
    }
    return <div key={i} className="my-2 rounded-lg border border-red-300 bg-red-50 p-2 text-sm text-red-800">
      Tool pipeline error: {event.message}
    </div>;
  };

  return (
    <div className="h-full w-full max-w-full max-h-full p-2 text-[15px] leading-7">
        {items.length === 0 && <div className="flex h-full min-h-64 items-center justify-center text-center text-[#8a928e]">
          <div><div className="mx-auto mb-3 h-2 w-2 rounded-full bg-[#58d69b]"/><p>Start speaking when you’re ready.</p><p className="mt-1 text-xs">Routing and tool activity will appear here.</p></div>
        </div>}
        {items.map((item, i) => item.kind === "text"
          ? <div key={i} className="my-3 max-w-[88%] rounded-2xl rounded-tl-md bg-[#f1f3f2] px-4 py-3 text-[#29332f]"><div className="mb-1 text-[10px] font-semibold uppercase tracking-[0.12em] text-[#7a847f]">PersonaPlex</div>{item.data.trim()}</div>
          : eventCard(item.data, i)
        )}
    </div>
  );
};
