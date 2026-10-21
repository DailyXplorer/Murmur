import { useEffect, useState } from "react";
import { commands, events, type LocalModelStatus } from "@/bindings";

/** The local models' state, shared by dictation and meetings, kept current
 * while they download. `null` until it is known. */
export const useLocalModel = (): LocalModelStatus | null => {
  const [status, setStatus] = useState<LocalModelStatus | null>(null);

  useEffect(() => {
    let disposed = false;
    void commands.getLocalModelStatus().then((result) => {
      if (!disposed && result.status === "ok") setStatus(result.data);
    });
    const unlisten = events.localModelStatus.listen((event) => {
      setStatus(event.payload);
    });
    return () => {
      disposed = true;
      void unlisten.then((stop) => stop());
    };
  }, []);

  return status;
};

/** Whole percent of a download, 0 when nothing downloads. */
export const downloadPercent = (status: LocalModelStatus | null): number => {
  const state = status?.state;
  if (state?.state !== "downloading" || state.total_bytes === 0) return 0;
  return Math.round((state.done_bytes / state.total_bytes) * 100);
};
