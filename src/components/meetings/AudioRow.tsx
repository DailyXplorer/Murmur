import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { WaveformIcon } from "@phosphor-icons/react/dist/csr/Waveform";
import { commands, type MeetingDetail } from "@/bindings";
import { formatSize } from "@/lib/meetings/format";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";

type Pending = "delete" | "retranscribe" | null;

/** Where the meeting's audio stands, with Transcribe Again and Delete Audio. */
export const AudioRow: React.FC<{
  meeting: MeetingDetail;
  onChanged: () => void;
}> = ({ meeting, onChanged }) => {
  const { t, i18n } = useTranslation();
  const [pending, setPending] = useState<Pending>(null);
  const finished = meeting.status === "ready" || meeting.status === "failed";

  const confirm = async () => {
    const action = pending;
    setPending(null);
    const result =
      action === "delete"
        ? await commands.deleteMeetingAudio(meeting.id)
        : await commands.retranscribeMeeting(meeting.id);
    if (result.status === "error") {
      toast.error(
        t(
          action === "delete"
            ? "meetings.audio.deleteError"
            : "meetings.audio.retranscribeError",
        ),
      );
    }
    onChanged();
  };

  const label =
    meeting.audio === "deleted"
      ? t("meetings.audio.deleted")
      : meeting.audio === "compressed"
        ? t("meetings.audio.compressed", {
            size: formatSize(meeting.audio_bytes, i18n.language),
          })
        : t("meetings.audio.tracks");

  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-t border-mid-gray/20 pt-3 pl-1">
      <p className="flex min-w-0 flex-1 items-center gap-2 text-xs text-mid-gray tabular-nums">
        <WaveformIcon size={15} className="shrink-0" aria-hidden="true" />
        <span className="min-w-0">{label}</span>
      </p>
      {finished && meeting.audio !== "deleted" && (
        <div className="flex shrink-0 items-center gap-2">
          <Button onClick={() => setPending("retranscribe")}>
            {t("meetings.audio.retranscribe")}
          </Button>
          <Button onClick={() => setPending("delete")}>
            {t("meetings.audio.delete")}
          </Button>
        </div>
      )}
      <Dialog
        open={pending !== null}
        onOpenChange={(open) => {
          if (!open) setPending(null);
        }}
        title={
          pending === "delete"
            ? t("meetings.audio.deleteDialog.title")
            : t("meetings.audio.retranscribeDialog.title")
        }
        closeLabel={t("common.close")}
        footer={
          <>
            <Button onClick={() => setPending(null)}>
              {t("meetings.cancel")}
            </Button>
            <Button onClick={() => void confirm()}>
              {pending === "delete"
                ? t("meetings.audio.delete")
                : t("meetings.audio.retranscribe")}
            </Button>
          </>
        }
      >
        <p className="text-sm text-text/80">
          {pending === "delete"
            ? t("meetings.audio.deleteDialog.description")
            : t("meetings.audio.retranscribeDialog.description")}
        </p>
      </Dialog>
    </div>
  );
};
