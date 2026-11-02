import { useEffect, useState } from "react";
import {
  commands,
  events,
  type LiveEntry,
  type LiveTranscript,
} from "@/bindings";

const keyOf = (entry: LiveEntry) =>
  `${entry.source}:${entry.start_ms}:${entry.end_ms}:${entry.kind}:${entry.text}`;

const merge = (current: LiveEntry[], added: LiveEntry[]): LiveEntry[] => {
  const seen = new Set(current.map(keyOf));
  const fresh = added.filter((entry) => {
    const key = keyOf(entry);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
  if (fresh.length === 0) return current;
  return [...current, ...fresh].sort(
    (a, b) => a.start_ms - b.start_ms || a.end_ms - b.end_ms,
  );
};

export const useLiveDraft = (
  id: number,
  enabled = true,
): LiveTranscript | null | undefined => {
  const [draft, setDraft] = useState<LiveTranscript | null | undefined>(
    undefined,
  );

  useEffect(() => {
    setDraft(undefined);
    if (!enabled) return;
    let disposed = false;
    let revision = 0;

    const pull = async () => {
      const asked = revision;
      try {
        const result = await commands.getLiveTranscript(id);
        if (disposed) return;
        if (result.status === "error") throw new Error(result.error);
        const pulled = result.data;
        const stale = revision !== asked;
        setDraft((current) => {
          if (pulled === null) return stale ? current : null;
          const entries = merge(current?.entries ?? [], pulled.entries);
          return {
            state: stale && current ? current.state : pulled.state,
            entries,
          };
        });
      } catch (error) {
        if (!disposed) console.error("Failed to load the live draft:", error);
      }
    };

    const live = events.meetingLiveUpdated.listen((event) => {
      if (disposed || event.payload.id !== id) return;
      revision++;
      const { state, appended } = event.payload;
      setDraft((current) => ({
        state,
        entries: merge(current?.entries ?? [], appended),
      }));
    });
    const updated = events.meetingsUpdated.listen((event) => {
      if (!disposed && event.payload.id === id) void pull();
    });
    void Promise.all([live, updated]).then(() => {
      if (!disposed) void pull();
    });
    return () => {
      disposed = true;
      void live.then((stop) => stop());
      void updated.then((stop) => stop());
    };
  }, [id, enabled]);

  return draft;
};
