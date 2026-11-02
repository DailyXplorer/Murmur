import React from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type LocalModelStatus } from "@/bindings";
import { downloadPercent } from "../../hooks/useLocalModel";
import { Button } from "../ui/Button";

export const LocalModelAction: React.FC<{
  models: LocalModelStatus | null;
}> = ({ models }) => {
  const { t } = useTranslation();
  const download = async () => {
    const result = await commands.downloadLocalModel();
    if (result.status === "error") toast.error(t("meetings.models.error"));
  };
  switch (models?.state.state) {
    case "missing":
    case "failed":
      return (
        <Button onClick={() => void download()}>
          {t("meetings.models.download")}
        </Button>
      );
    case "downloading":
      return (
        <span className="text-sm text-text/60 tabular-nums">
          {t("meetings.models.downloading", {
            percent: downloadPercent(models),
          })}
        </span>
      );
    case "preparing":
      return (
        <span className="text-sm text-text/60">
          {t("meetings.models.preparing")}
        </span>
      );
    default:
      return null;
  }
};
