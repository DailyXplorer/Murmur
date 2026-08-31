import React, { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { ArrowCounterClockwiseIcon } from "@phosphor-icons/react/dist/csr/ArrowCounterClockwise";
import { CheckIcon } from "@phosphor-icons/react/dist/csr/Check";
import { CopyIcon } from "@phosphor-icons/react/dist/csr/Copy";
import { FolderOpenIcon } from "@phosphor-icons/react/dist/csr/FolderOpen";
import { StarIcon } from "@phosphor-icons/react/dist/csr/Star";
import { TrashIcon } from "@phosphor-icons/react/dist/csr/Trash";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  commands,
  events,
  type HistoryEntry,
  type HistoryUpdatePayload,
} from "@/bindings";
import { formatDateTime } from "@/utils/dateFormat";
import { AudioPlayer, AudioPlayerGroup } from "../../ui/AudioPlayer";
import { Button } from "../../ui/Button";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingsPage } from "../../ui/SettingsPage";
import { HistoryLimit } from "../HistoryLimit";
import { RecordingRetentionPeriodSelector } from "../RecordingRetentionPeriod";

const IconButton: React.FC<{
  onClick: () => void;
  title: string;
  disabled?: boolean;
  active?: boolean;
  children: React.ReactNode;
}> = ({ onClick, title, disabled, active, children }) => (
  <button
    type="button"
    onClick={onClick}
    disabled={disabled}
    aria-label={title}
    className={`flex size-10 cursor-pointer items-center justify-center rounded-md transition-[color,background-color,transform] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary active:scale-[0.96] disabled:cursor-not-allowed disabled:text-text/20 disabled:active:scale-100 ${
      active
        ? "bg-logo-primary/10 text-logo-primary hover:bg-logo-primary/20 hover:text-logo-primary/80"
        : "text-text/50 hover:bg-mid-gray/10 hover:text-logo-primary"
    }`}
    title={title}
  >
    {children}
  </button>
);

const PAGE_SIZE = 30;

interface PendingHistoryEntry {
  entry: HistoryEntry;
  revision: number;
}

interface OpenRecordingsButtonProps {
  onClick: () => void;
  label: string;
}

const OpenRecordingsButton: React.FC<OpenRecordingsButtonProps> = ({
  onClick,
  label,
}) => (
  <Button
    onClick={onClick}
    variant="secondary"
    size="sm"
    className="flex items-center gap-2"
    title={label}
  >
    <FolderOpenIcon size={15} aria-hidden="true" />
    <span>{label}</span>
  </Button>
);

const upsertHistoryEntry = (
  entries: HistoryEntry[],
  entry: HistoryEntry,
  moveToStart: boolean,
): HistoryEntry[] => {
  const existingIndex = entries.findIndex(
    (existingEntry) => existingEntry.id === entry.id,
  );

  if (existingIndex === -1 || moveToStart) {
    return [
      entry,
      ...entries.filter((existingEntry) => existingEntry.id !== entry.id),
    ];
  }

  return entries.map((existingEntry) =>
    existingEntry.id === entry.id ? entry : existingEntry,
  );
};

const mergeHistoryPage = ({
  currentEntries,
  pageEntries,
  isFirstPage,
  pendingEntries,
}: {
  currentEntries: HistoryEntry[];
  pageEntries: HistoryEntry[];
  isFirstPage: boolean;
  pendingEntries: PendingHistoryEntry[];
}): HistoryEntry[] => {
  const pendingEntriesById = new Map(
    pendingEntries.map(({ entry }) => [entry.id, entry]),
  );
  const pageEntryIds = new Set(pageEntries.map((entry) => entry.id));
  const pendingEntriesOutsidePage = pendingEntries
    .filter(({ entry }) => !pageEntryIds.has(entry.id))
    .sort((left, right) => right.revision - left.revision)
    .map(({ entry }) => entry);
  const pageEntriesWithPendingUpdates = pageEntries.map(
    (entry) => pendingEntriesById.get(entry.id) ?? entry,
  );

  if (isFirstPage) {
    return [...pendingEntriesOutsidePage, ...pageEntriesWithPendingUpdates];
  }

  const mergedEntries: HistoryEntry[] = [];
  const seenEntryIds = new Set<number>();
  for (const entry of [
    ...currentEntries,
    ...pendingEntriesOutsidePage,
    ...pageEntriesWithPendingUpdates,
  ]) {
    if (seenEntryIds.has(entry.id)) {
      continue;
    }

    seenEntryIds.add(entry.id);
    mergedEntries.push(pendingEntriesById.get(entry.id) ?? entry);
  }

  return mergedEntries;
};

const stopHistoryUpdateListener = async (stopListening: () => void) => {
  try {
    await stopListening();
  } catch (error) {
    console.error("Failed to stop history update listener:", error);
  }
};

export const HistorySettings: React.FC = () => {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [hasMore, setHasMore] = useState(true);
  const [historyListenerReady, setHistoryListenerReady] = useState(false);
  const [hasQueuedFirstPageRefresh, setHasQueuedFirstPageRefresh] =
    useState(false);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const entriesRef = useRef<HistoryEntry[]>([]);
  const loadingRef = useRef(false);
  const historyEventRevisionRef = useRef(0);
  const activeHistoryRequestRevisionRef = useRef<number | null>(null);
  const queuedFirstPageRefreshRef = useRef(false);
  const pendingHistoryEntriesByIdRef = useRef<Map<number, PendingHistoryEntry>>(
    new Map(),
  );

  // Keep ref in sync for use in IntersectionObserver callback
  useEffect(() => {
    entriesRef.current = entries;
  }, [entries]);

  const loadPage = useCallback(async (cursor?: number) => {
    const isFirstPage = cursor === undefined;
    if (loadingRef.current) {
      if (isFirstPage) {
        queuedFirstPageRefreshRef.current = true;
      }
      return;
    }
    loadingRef.current = true;
    const requestRevision = historyEventRevisionRef.current;
    activeHistoryRequestRevisionRef.current = requestRevision;
    pendingHistoryEntriesByIdRef.current.clear();

    if (isFirstPage) setLoading(true);

    try {
      const result = await commands.getHistoryEntries(
        cursor ?? null,
        PAGE_SIZE,
      );
      if (result.status === "ok") {
        const { entries: newEntries, has_more } = result.data;
        const pendingEntries = Array.from(
          pendingHistoryEntriesByIdRef.current.values(),
        ).filter(({ revision }) => revision > requestRevision);
        setEntries((prev) =>
          mergeHistoryPage({
            currentEntries: prev,
            pageEntries: newEntries,
            isFirstPage,
            pendingEntries,
          }),
        );
        setHasMore(has_more);
      }
    } catch (error) {
      console.error("Failed to load history entries:", error);
    } finally {
      setLoading(false);
      loadingRef.current = false;
      if (activeHistoryRequestRevisionRef.current === requestRevision) {
        activeHistoryRequestRevisionRef.current = null;
        pendingHistoryEntriesByIdRef.current.clear();
      }
      if (queuedFirstPageRefreshRef.current) {
        queuedFirstPageRefreshRef.current = false;
        setHasQueuedFirstPageRefresh(true);
      }
    }
  }, []);

  // Initial load
  useEffect(() => {
    if (historyListenerReady) {
      void loadPage();
    }
  }, [historyListenerReady, loadPage]);

  useEffect(() => {
    if (!hasQueuedFirstPageRefresh) return;

    setHasQueuedFirstPageRefresh(false);
    void loadPage();
  }, [hasQueuedFirstPageRefresh, loadPage]);

  // Infinite scroll via IntersectionObserver
  useEffect(() => {
    if (loading) return;

    const sentinel = sentinelRef.current;
    if (!sentinel || !hasMore) return;

    const observer = new IntersectionObserver(
      (observerEntries) => {
        const first = observerEntries[0];
        if (first.isIntersecting) {
          const lastEntry = entriesRef.current[entriesRef.current.length - 1];
          if (lastEntry) {
            loadPage(lastEntry.id);
          }
        }
      },
      { threshold: 0 },
    );

    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [loading, hasMore, loadPage]);

  // Listen for new entries added from the transcription pipeline
  useEffect(() => {
    let disposed = false;
    const unlisten = events.historyUpdatePayload
      .listen((event) => {
        const payload: HistoryUpdatePayload = event.payload;
        if (payload.action === "added") {
          const revision = historyEventRevisionRef.current + 1;
          historyEventRevisionRef.current = revision;
          if (activeHistoryRequestRevisionRef.current !== null) {
            pendingHistoryEntriesByIdRef.current.set(payload.entry.id, {
              entry: payload.entry,
              revision,
            });
          }
          setEntries((prev) => upsertHistoryEntry(prev, payload.entry, true));
        } else if (payload.action === "updated") {
          const revision = historyEventRevisionRef.current + 1;
          historyEventRevisionRef.current = revision;
          if (activeHistoryRequestRevisionRef.current !== null) {
            pendingHistoryEntriesByIdRef.current.set(payload.entry.id, {
              entry: payload.entry,
              revision,
            });
          }
          setEntries((prev) => upsertHistoryEntry(prev, payload.entry, false));
        }
        // "deleted" and "toggled" are handled by optimistic updates only,
        // so we intentionally ignore them here to avoid double-mutation.
      })
      .then((stopListening) => {
        if (disposed) {
          void stopHistoryUpdateListener(stopListening);
          return undefined;
        }

        setHistoryListenerReady(true);
        return stopListening;
      })
      .catch((error) => {
        console.error("Failed to listen for history updates:", error);
        if (!disposed) {
          setHistoryListenerReady(true);
        }
        return undefined;
      });

    return () => {
      disposed = true;
      void unlisten.then((stopListening) => {
        if (stopListening) {
          return stopHistoryUpdateListener(stopListening);
        }
      });
    };
  }, []);

  const toggleSaved = async (id: number) => {
    // Optimistic update
    setEntries((prev) =>
      prev.map((e) => (e.id === id ? { ...e, saved: !e.saved } : e)),
    );
    try {
      const result = await commands.toggleHistoryEntrySaved(id);
      if (result.status !== "ok") {
        // Revert on failure
        setEntries((prev) =>
          prev.map((e) => (e.id === id ? { ...e, saved: !e.saved } : e)),
        );
      }
    } catch (error) {
      console.error("Failed to toggle saved status:", error);
      // Revert on failure
      setEntries((prev) =>
        prev.map((e) => (e.id === id ? { ...e, saved: !e.saved } : e)),
      );
    }
  };

  const copyToClipboard = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch (error) {
      console.error("Failed to copy to clipboard:", error);
    }
  };

  const getAudioUrl = useCallback(async (id: number) => {
    try {
      const result = await commands.getAudioFilePath(id);
      if (result.status === "ok") {
        return convertFileSrc(result.data, "asset");
      }
      return null;
    } catch (error) {
      console.error("Failed to get audio file path:", error);
      return null;
    }
  }, []);

  const deleteAudioEntry = async (id: number) => {
    // Optimistically remove
    setEntries((prev) => prev.filter((e) => e.id !== id));
    try {
      const result = await commands.deleteHistoryEntry(id);
      if (result.status !== "ok") {
        // Reload on failure
        loadPage();
      }
    } catch (error) {
      console.error("Failed to delete entry:", error);
      loadPage();
    }
  };

  const retryHistoryEntry = async (id: number) => {
    const result = await commands.retryHistoryEntryTranscription(id);
    if (result.status !== "ok") {
      throw new Error(String(result.error));
    }
  };

  const openRecordingsFolder = async () => {
    try {
      const result = await commands.openRecordingsFolder();
      if (result.status !== "ok") {
        throw new Error(String(result.error));
      }
    } catch (error) {
      console.error("Failed to open recordings folder:", error);
    }
  };

  let content: React.ReactNode;

  if (loading) {
    content = (
      <div className="px-4 py-3 text-center text-text/60">
        {t("settings.history.loading")}
      </div>
    );
  } else if (entries.length === 0) {
    content = (
      <div className="px-4 py-3 text-center text-text/60">
        {t("settings.history.empty")}
      </div>
    );
  } else {
    content = (
      <>
        <AudioPlayerGroup>
          <div className="divide-y divide-mid-gray/20">
            {entries.map((entry) => (
              <HistoryEntryComponent
                key={entry.id}
                entry={entry}
                onToggleSaved={() => toggleSaved(entry.id)}
                onCopyText={() => copyToClipboard(entry.transcription_text)}
                getAudioUrl={getAudioUrl}
                deleteAudio={deleteAudioEntry}
                retryTranscription={retryHistoryEntry}
              />
            ))}
          </div>
        </AudioPlayerGroup>
      </>
    );
  }

  return (
    <SettingsPage label={t("sidebar.history")}>
      <SettingsGroup title={t("settings.history.preferences")}>
        <HistoryLimit descriptionMode="tooltip" grouped={true} />
        <RecordingRetentionPeriodSelector
          descriptionMode="tooltip"
          grouped={true}
        />
      </SettingsGroup>

      <SettingsGroup
        title={t("settings.history.title")}
        action={
          <OpenRecordingsButton
            onClick={openRecordingsFolder}
            label={t("settings.history.openFolder")}
          />
        }
      >
        {content}
      </SettingsGroup>

      {!loading && hasMore && entries.length > 0 && (
        <div ref={sentinelRef} className="h-1" />
      )}
    </SettingsPage>
  );
};

interface HistoryEntryProps {
  entry: HistoryEntry;
  onToggleSaved: () => void;
  onCopyText: () => void;
  getAudioUrl: (id: number) => Promise<string | null>;
  deleteAudio: (id: number) => Promise<void>;
  retryTranscription: (id: number) => Promise<void>;
}

const HistoryEntryComponent: React.FC<HistoryEntryProps> = ({
  entry,
  onToggleSaved,
  onCopyText,
  getAudioUrl,
  deleteAudio,
  retryTranscription,
}) => {
  const { t, i18n } = useTranslation();
  const [showCopied, setShowCopied] = useState(false);
  const [retrying, setRetrying] = useState(false);

  const hasTranscription = entry.transcription_text.trim().length > 0;

  const handleLoadAudio = useCallback(
    () => getAudioUrl(entry.id),
    [getAudioUrl, entry.id],
  );

  const handleCopyText = () => {
    if (!hasTranscription) {
      return;
    }

    onCopyText();
    setShowCopied(true);
    setTimeout(() => setShowCopied(false), 2000);
  };

  const handleDeleteEntry = async () => {
    try {
      await deleteAudio(entry.id);
    } catch (error) {
      console.error("Failed to delete entry:", error);
      toast.error(t("settings.history.deleteError"));
    }
  };

  const handleRetranscribe = async () => {
    try {
      setRetrying(true);
      await retryTranscription(entry.id);
    } catch (error) {
      console.error("Failed to re-transcribe:", error);
      toast.error(t("settings.history.retranscribeError"));
    } finally {
      setRetrying(false);
    }
  };

  const formattedDate = formatDateTime(String(entry.timestamp), i18n.language);

  return (
    <div className="px-4 py-2 pb-5 flex flex-col gap-3">
      <div className="flex justify-between items-center">
        <p className="text-sm font-medium">{formattedDate}</p>
        <div className="flex items-center">
          <IconButton
            onClick={handleCopyText}
            disabled={!hasTranscription || retrying}
            title={t("settings.history.copyToClipboard")}
          >
            {showCopied ? (
              <CheckIcon size={15} aria-hidden="true" />
            ) : (
              <CopyIcon size={15} aria-hidden="true" />
            )}
          </IconButton>
          <IconButton
            onClick={onToggleSaved}
            disabled={retrying}
            active={entry.saved}
            title={
              entry.saved
                ? t("settings.history.unsave")
                : t("settings.history.save")
            }
          >
            <StarIcon
              size={15}
              weight={entry.saved ? "fill" : "light"}
              aria-hidden="true"
            />
          </IconButton>
          <IconButton
            onClick={handleRetranscribe}
            disabled={retrying}
            title={t("settings.history.retranscribe")}
          >
            <ArrowCounterClockwiseIcon
              size={15}
              aria-hidden="true"
              style={
                retrying
                  ? { animation: "spin 1s linear infinite reverse" }
                  : undefined
              }
            />
          </IconButton>
          <IconButton
            onClick={handleDeleteEntry}
            disabled={retrying}
            title={t("settings.history.delete")}
          >
            <TrashIcon size={15} aria-hidden="true" />
          </IconButton>
        </div>
      </div>

      <p
        className={`italic text-sm pb-2 ${
          retrying
            ? ""
            : hasTranscription
              ? "text-text/90 select-text cursor-text whitespace-pre-wrap break-words"
              : "text-text/40"
        }`}
        style={
          retrying
            ? { animation: "transcribe-pulse 3s ease-in-out infinite" }
            : undefined
        }
      >
        {retrying && (
          <style>{`
            @keyframes transcribe-pulse {
              0%, 100% { color: color-mix(in srgb, var(--color-text) 40%, transparent); }
              50% { color: color-mix(in srgb, var(--color-text) 90%, transparent); }
            }
          `}</style>
        )}
        {retrying
          ? t("settings.history.transcribing")
          : hasTranscription
            ? entry.transcription_text
            : t("settings.history.transcriptionFailed")}
      </p>

      <AudioPlayer onLoadRequest={handleLoadAudio} className="w-full" />
    </div>
  );
};
