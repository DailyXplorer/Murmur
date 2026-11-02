import { useCallback, useEffect, useState } from "react";
import type { Result } from "@/bindings";

const DEBOUNCE_MS = 200;
/** Shorter queries show the normal list instead. */
const MIN_QUERY_LENGTH = 2;

export type SearchState<Hit> =
  | { status: "idle" }
  /** `hits` are the previous answer's, kept on screen while typing. */
  | { status: "loading"; hits: Hit[] }
  | { status: "results"; hits: Hit[] }
  | { status: "error" };

interface Answer<Hit> {
  query: string;
  /** `null` when the search failed. */
  hits: Hit[] | null;
}

/**
 * Runs `search` on `query` once typing pauses. Only the answer to the latest
 * query is kept: an earlier one arriving late is dropped.
 */
export const useSearch = <Hit>(
  query: string,
  search: (query: string) => Promise<Result<Hit[], string>>,
) => {
  const trimmed = query.trim();
  const active = trimmed.length >= MIN_QUERY_LENGTH;
  const [answer, setAnswer] = useState<Answer<Hit> | null>(null);
  const [round, setRound] = useState(0);
  // A cleared field forgets its results, so the next query starts blank.
  if (!active && answer !== null) setAnswer(null);

  useEffect(() => {
    if (!active) return;
    let stale = false;
    const timer = window.setTimeout(async () => {
      let hits: Hit[] | null = null;
      try {
        const result = await search(trimmed);
        if (result.status === "error") throw new Error(result.error);
        hits = result.data;
      } catch (error) {
        if (!stale) console.error("Search failed:", error);
      }
      if (!stale) setAnswer({ query: trimmed, hits });
    }, DEBOUNCE_MS);
    return () => {
      stale = true;
      window.clearTimeout(timer);
    };
  }, [active, trimmed, search, round]);

  const state: SearchState<Hit> = !active
    ? { status: "idle" }
    : answer?.query !== trimmed
      ? { status: "loading", hits: answer?.hits ?? [] }
      : answer.hits === null
        ? { status: "error" }
        : { status: "results", hits: answer.hits };

  /** Changes the hits shown, after an action on one of them. */
  const updateHits = useCallback(
    (change: (hits: Hit[]) => Hit[]) =>
      setAnswer((current) =>
        current?.hits ? { ...current, hits: change(current.hits) } : current,
      ),
    [],
  );

  /** Searches again, keeping the current hits on screen meanwhile. */
  const refresh = useCallback(() => setRound((value) => value + 1), []);

  return { state, updateHits, refresh };
};
