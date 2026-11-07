import React from "react";
import { useTranslation } from "react-i18next";
import type { Engine } from "@/bindings";

/** Where a transcript was made: "On this Mac" / "On this PC" or "ChatGPT". */
export const EngineLabel: React.FC<{ engine: Engine }> = ({ engine }) => {
  const { t } = useTranslation();
  return (
    <>
      {engine === "local"
        ? t("settings.transcription.engine.local.title")
        : t("settings.history.engineCloud")}
    </>
  );
};
