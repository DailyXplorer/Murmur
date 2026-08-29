import React from "react";
import { useTranslation } from "react-i18next";
import { Button } from "./Button";

interface PathDisplayProps {
  path: string;
  onOpen: () => void;
  disabled?: boolean;
}

export const PathDisplay: React.FC<PathDisplayProps> = ({
  path,
  onOpen,
  disabled = false,
}) => {
  const { t } = useTranslation();

  return (
    <div className="flex w-full flex-col gap-2">
      <div
        data-slot="path-surface"
        className="w-full min-w-0 px-2 py-2 bg-mid-gray/10 border border-mid-gray/80 rounded-lg text-xs font-sans break-all select-text cursor-text"
      >
        {path}
      </div>
      <div data-slot="path-action" className="flex justify-end">
        <Button
          onClick={onOpen}
          variant="secondary"
          size="sm"
          disabled={disabled}
          className="px-3 py-2"
        >
          {t("common.open")}
        </Button>
      </div>
    </div>
  );
};
