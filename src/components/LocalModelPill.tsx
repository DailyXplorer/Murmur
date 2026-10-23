import React from "react";
import { useTranslation } from "react-i18next";
import type { LocalModelStatus } from "@/bindings";
import { downloadPercent } from "@/hooks/useLocalModel";

/** A slim line saying the local model is still on its way, shown while
 * nothing else on screen does. Hidden once it is ready or stopped. */
export const LocalModelPill: React.FC<{
  status: LocalModelStatus | null;
  className?: string;
}> = ({ status, className = "" }) => {
  const { t } = useTranslation();
  const state = status?.state.state;
  if (state !== "downloading" && state !== "preparing") return null;

  return (
    <div className={className}>
      <span
        role="status"
        className="inline-flex h-7 items-center rounded-full border border-mid-gray/20 bg-background px-3 text-xs text-text/70 tabular-nums"
      >
        {state === "downloading"
          ? t("localModelPill.downloading", {
              percent: downloadPercent(status),
            })
          : t("localModelPill.preparing")}
      </span>
    </div>
  );
};
