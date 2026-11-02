import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  commands,
  events,
  type LiveState,
  type MeetingRuntime,
} from "@/bindings";
import { formatDuration } from "@/lib/meetings/format";
import { useLocalModel } from "../../hooks/useLocalModel";
import { useNow } from "../../hooks/useNow";
import { useSettings } from "../../hooks/useSettings";
import { Button } from "../ui/Button";
import { LIVE_LABEL_KEYS, LiveEntries } from "./LiveEntries";
import { LocalModelAction } from "./LocalModelAction";
import { BackButton } from "./MeetingView";
import { useLiveDraft } from "./useLiveDraft";

const NOTICE_CLASSES =
  "flex items-center gap-3 rounded-xl border border-mid-gray/20 px-4 py-3";

const UnavailableNotice: React.FC<{
  reason: Extract<LiveState, { state: "unavailable" }>["reason"];
}> = ({ reason }) => {
  const { t } = useTranslation();
  const models = useLocalModel();
  if (
    reason === "missing" ||
    reason === "downloading" ||
    reason === "preparing"
  ) {
    return (
      <div role="status" className={NOTICE_CLASSES}>
        <p className="flex-1 text-sm">{t("meetings.live.unavailable")}</p>
        <LocalModelAction models={models} />
      </div>
    );
  }
  const message = {
    off: "meetings.live.off",
    unsupported: "meetings.live.unsupported",
    engine_failed: "meetings.live.failed",
  }[reason];
  return (
    <div
      role={reason === "engine_failed" ? "alert" : "status"}
      className={NOTICE_CLASSES}
    >
      <p className="flex-1 text-sm">{t(message)}</p>
    </div>
  );
};

export const LiveMeetingView: React.FC<{
  id: number;
  onBack: () => void;
  onStopped: () => void;
}> = ({ id, onBack, onStopped }) => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const [startedAt, setStartedAt] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const draft = useLiveDraft(id);
  const now = useNow(startedAt !== null);

  useEffect(() => {
    let disposed = false;
    let revision = 0;
    const apply = (runtime: MeetingRuntime) => {
      if (runtime.state === "recording" && runtime.id === id) {
        setStartedAt(runtime.started_at_ms);
      } else {
        onStopped();
      }
    };
    const changed = events.meetingStateChanged.listen((event) => {
      if (disposed) return;
      revision++;
      apply(event.payload);
    });
    void changed.then(async () => {
      const asked = revision;
      const runtime = await commands.getMeetingRuntime();
      if (!disposed && revision === asked) apply(runtime);
    });
    return () => {
      disposed = true;
      void changed.then((stop) => stop());
    };
  }, [id, onStopped]);

  const stop = async (event: React.MouseEvent) => {
    if (event.detail > 1) return;
    setBusy(true);
    try {
      const result = await commands.stopMeeting();
      if (result.status === "error") toast.error(t("meetings.stopError"));
    } catch (error) {
      console.error("Meeting action failed:", error);
      toast.error(t("meetings.stopError"));
    } finally {
      setBusy(false);
    }
  };

  const entries = draft?.entries ?? [];
  const speech = entries.filter((entry) => entry.kind === "speech");
  const copyAll = () => {
    const text = speech
      .map((entry) => `${t(LIVE_LABEL_KEYS[entry.source])}: ${entry.text}`)
      .join("\n");
    navigator.clipboard.writeText(text).then(
      () => toast.success(t("meetings.transcript.copied")),
      (error: unknown) => {
        console.error("Failed to copy the live transcript:", error);
        toast.error(t("meetings.transcript.copyError"));
      },
    );
  };

  const state: LiveState = draft?.state ?? { state: "starting" };
  const listening =
    state.state === "starting" ||
    state.state === "live" ||
    state.state === "catching_up";
  const pasteUnbound =
    getSetting("bindings")?.meeting_paste_new?.current_binding === "";

  return (
    <article
      aria-label={t("meetings.live.title")}
      className="mx-auto w-full max-w-3xl space-y-4 pb-1"
    >
      <BackButton onBack={onBack} />
      <div className="space-y-2">
        <h1 className="px-1 font-title text-[28px] font-normal leading-tight text-text">
          {t("meetings.live.title")}
        </h1>
        <div className="flex items-center gap-2">
          <span className="ml-1 inline-flex rounded-md border border-mid-gray/20 px-2 py-0.5 text-xs text-mid-gray tabular-nums">
            {t("meetings.recording.active", {
              time: formatDuration(startedAt === null ? 0 : now - startedAt),
            })}
          </span>
          <span className="flex-1" />
          <Button disabled={speech.length === 0} onClick={copyAll}>
            {t("meetings.live.copyAll")}
          </Button>
          <Button disabled={busy} onClick={(event) => void stop(event)}>
            {t("meetings.stop")}
          </Button>
        </div>
      </div>
      {state.state === "catching_up" && (
        <p
          role="status"
          className="rounded-xl border border-mid-gray/20 px-4 py-1.5 text-xs text-text/80"
        >
          {t("meetings.live.catchingUp")}
        </p>
      )}
      {state.state === "unavailable" && (
        <UnavailableNotice reason={state.reason} />
      )}
      {pasteUnbound && (
        <p className="px-1 text-xs text-text/60">
          {t("meetings.live.pasteHint")}
        </p>
      )}
      {entries.length > 0 ? (
        <LiveEntries entries={entries} />
      ) : (
        listening && (
          <p className="px-1 py-6 text-center text-sm text-mid-gray">
            {t("meetings.live.starting")}
          </p>
        )
      )}
    </article>
  );
};
