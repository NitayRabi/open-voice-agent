import { useCallback, useEffect, useState } from "react";
import { useSocketContext } from "../SocketContext";
import { decodeMessage } from "../../../protocol/encoder";
import { ToolEvent } from "../../../protocol/types";

export type TranscriptItem =
  | { kind: "text"; data: string }
  | { kind: "toolEvent"; data: ToolEvent };

export const useServerText = () => {
  const [text, setText] = useState<string[]>([]);
  const [items, setItems] = useState<TranscriptItem[]>([]);
  const [totalTextMessages, setTotalTextMessages] = useState(0);
  const { socket } = useSocketContext();

  const onSocketMessage = useCallback((e: MessageEvent) => {
    const dataArray = new Uint8Array(e.data);
    const message = decodeMessage(dataArray);
    if (message.type === "text") {
      setText(text => [...text, message.data]);
      setItems(items => {
        const last = items[items.length - 1];
        if (last?.kind === "text") {
          return [...items.slice(0, -1), { kind: "text", data: last.data + message.data }];
        }
        return [...items, { kind: "text", data: message.data }];
      });
      setTotalTextMessages(count => count + 1);
    } else if (message.type === "toolEvent") {
      setItems(items => [...items, { kind: "toolEvent", data: message.data }]);
    }
  }, []);

  useEffect(() => {
    const currentSocket = socket;
    if (!currentSocket) {
      return;
    }
    setText([]);
    setItems([]);
    currentSocket.addEventListener("message", onSocketMessage);
    return () => {
      currentSocket.removeEventListener("message", onSocketMessage);
    };
  }, [socket]);

  return { text, items, totalTextMessages };
};
