import React, { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type MeetingSpeaker } from "@/bindings";
import { speakerColor } from "@/lib/meetings/speakerColors";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { Input } from "../ui/Input";

const MAX_NAME_LENGTH = 60;

interface RenameSpeakersDialogProps {
  meetingId: number;
  speakers: MeetingSpeaker[];
  /** The speaker whose field gets the focus, or `null` for the first. */
  focusKey: string | null;
  open: boolean;
  onClose: () => void;
  onSaved: () => void;
}

/** One field per speaker; a new name shows everywhere in the meeting. */
export const RenameSpeakersDialog: React.FC<RenameSpeakersDialogProps> = ({
  meetingId,
  speakers,
  focusKey,
  open,
  onClose,
  onSaved,
}) => {
  const { t } = useTranslation();
  const [names, setNames] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);
  const [wasOpen, setWasOpen] = useState(false);
  const focusRef = useRef<HTMLInputElement>(null);

  // Filled as the dialog opens only: the meeting reloads while it is open
  // (summary or compression done), and that must not wipe what is typed.
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setNames(
        Object.fromEntries(speakers.map(({ key, name }) => [key, name])),
      );
    }
  }

  const changed = speakers.filter(
    ({ key, name }) =>
      names[key] !== undefined && names[key].trim() !== name.trim(),
  );
  const valid = speakers.every(({ key }) => (names[key] ?? "x").trim() !== "");

  const save = async () => {
    setSaving(true);
    try {
      for (const { key } of changed) {
        const result = await commands.renameSpeaker(
          meetingId,
          key,
          names[key].trim(),
        );
        if (result.status === "error") throw new Error(result.error);
      }
      onClose();
    } catch (error) {
      console.error("Failed to rename a speaker:", error);
      toast.error(t("meetings.renameDialog.error"));
    } finally {
      setSaving(false);
      onSaved();
    }
  };

  const focusIndex = Math.max(
    0,
    speakers.findIndex(({ key }) => key === focusKey),
  );

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
      title={t("meetings.renameDialog.title")}
      description={t("meetings.renameDialog.description")}
      closeLabel={t("common.close")}
      initialFocusRef={focusRef}
      footer={
        <>
          <Button onClick={onClose}>{t("meetings.cancel")}</Button>
          <Button
            disabled={saving || !valid || changed.length === 0}
            onClick={() => void save()}
          >
            {t("meetings.renameDialog.save")}
          </Button>
        </>
      }
    >
      <form
        className="space-y-2"
        onSubmit={(event) => {
          event.preventDefault();
          if (valid && changed.length > 0 && !saving) void save();
        }}
      >
        {speakers.map((speaker, index) => (
          <label key={speaker.key} className="flex items-center gap-3">
            <span
              aria-hidden="true"
              className="size-2.5 shrink-0 rounded-full"
              style={{ backgroundColor: speakerColor(speaker.color_index) }}
            />
            <Input
              ref={index === focusIndex ? focusRef : undefined}
              className="flex-1"
              value={names[speaker.key] ?? ""}
              maxLength={MAX_NAME_LENGTH}
              aria-label={t("meetings.renameDialog.nameLabel", {
                name: speaker.default_name,
              })}
              onChange={(event) =>
                setNames((current) => ({
                  ...current,
                  [speaker.key]: event.target.value,
                }))
              }
            />
          </label>
        ))}
        <button type="submit" hidden />
      </form>
    </Dialog>
  );
};
