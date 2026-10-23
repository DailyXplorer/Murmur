import { useEffect, useState } from "react";
import { commands, events, type LocalModelStatus } from "@/bindings";

/** The local models' state, shared by dictation and meetings, kept current
 * while they download. `null` until it is known. */
export const useLocalModel = (): LocalModelStatus | null => {
  const [status, setStatus] = useState<LocalModelStatus | null>(null);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let revision = 0;

    const initialize = async () => {
      try {
        // Subscribe first so a transition during setup is either delivered
        // as an event or included in the subsequent snapshot.
        const stop = await events.localModelStatus.listen((event) => {
          if (disposed) return;
          revision++;
          setStatus(event.payload);
        });
        if (disposed) {
          stop();
          return;
        }
        unlisten = stop;
        const initialRevision = revision;
        const result = await commands.getLocalModelStatus();
        // An event received while the snapshot was in flight is newer.
        if (
          !disposed &&
          revision === initialRevision &&
          result.status === "ok"
        ) {
          setStatus(result.data);
        }
      } catch (error) {
        if (!disposed)
          console.error("Failed to load local model status", error);
      }
    };
    void initialize();
    return () => {
      disposed = true;
      unlisten?.();
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
