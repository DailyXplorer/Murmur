import React, { useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { ShareCard } from "./ShareCard";
import { renderShareCard } from "./renderShareCard";
import { SHARE_SIZE, type ShareStats } from "./shareStats";

/** Room the dialog's header, footer and padding take, besides the card. */
const CHROME_REM = 15;

/** The side the preview can have: the dialog's width, and the window's
 * height once the dialog's own parts are taken off. */
const previewSide = (width: number) =>
  Math.max(
    0,
    Math.min(
      width,
      window.innerHeight -
        CHROME_REM *
          parseFloat(getComputedStyle(document.documentElement).fontSize),
    ),
  );

const Preview: React.FC<{
  stats: ShareStats;
  cardRef: React.Ref<HTMLDivElement>;
}> = ({ stats, cardRef }) => {
  const frameRef = useRef<HTMLDivElement>(null);
  const [side, setSide] = useState(0);

  useLayoutEffect(() => {
    const frame = frameRef.current;
    if (!frame) return;
    const measure = () => setSide(previewSide(frame.clientWidth));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(frame);
    window.addEventListener("resize", measure);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, []);

  return (
    <div ref={frameRef} className="flex w-full justify-center">
      <div
        className="relative overflow-hidden rounded-lg border border-mid-gray/20"
        style={{ width: side, height: side }}
      >
        {/* The card is drawn at its real size and scaled down for the
            preview, so the picture is exactly what is shown. */}
        <div
          style={{
            width: SHARE_SIZE,
            height: SHARE_SIZE,
            transform: `scale(${side / SHARE_SIZE})`,
            transformOrigin: "top left",
          }}
        >
          <ShareCard stats={stats} ref={cardRef} />
        </div>
      </div>
    </div>
  );
};

type Action = "save" | "copy";

/**
 * The one share dialog of Insights: the square card, Save PNG and Copy
 * image. No options: the card is always the same light card.
 */
export const ShareDialog: React.FC<{
  open: boolean;
  stats: ShareStats;
  onOpenChange: (open: boolean) => void;
}> = ({ open, stats, onOpenChange }) => {
  const { t } = useTranslation();
  const cardRef = useRef<HTMLDivElement>(null);
  const [busy, setBusy] = useState<Action | null>(null);

  const run = async (action: Action) => {
    const card = cardRef.current;
    if (!card || busy) return;
    setBusy(action);
    try {
      const png = await renderShareCard(card);
      const result =
        action === "copy"
          ? await commands.copyInsightsCard(png)
          : await commands.saveInsightsCard(png);
      if (result.status === "error") throw new Error(result.error);
      // Saving returns no path when the user cancels the dialog.
      if (action === "copy" || result.data) {
        toast.success(t(`insights.share.${action}.done`));
      }
    } catch (error) {
      console.error(`Failed to ${action} the insights card:`, error);
      toast.error(t(`insights.share.${action}.error`));
    } finally {
      setBusy(null);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("insights.share.title")}
      closeLabel={t("common.close")}
      contentFades={false}
      footer={
        <>
          <Button disabled={busy !== null} onClick={() => void run("save")}>
            {t("insights.share.save.button")}
          </Button>
          <Button disabled={busy !== null} onClick={() => void run("copy")}>
            {t("insights.share.copy.button")}
          </Button>
        </>
      }
    >
      <Preview stats={stats} cardRef={cardRef} />
    </Dialog>
  );
};
