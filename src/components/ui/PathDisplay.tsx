import React from "react";
import { useTranslation } from "react-i18next";
import { Button } from "./Button";
import { Input } from "./Input";

interface PathDisplayProps {
  path: string;
  label: string;
  onOpen: () => void;
  disabled?: boolean;
}

export const PathDisplay: React.FC<PathDisplayProps> = ({
  path,
  label,
  onOpen,
  disabled = false,
}) => {
  const { t } = useTranslation();

  return (
    <div className="flex items-center gap-2">
      <Input
        readOnly
        value={path}
        title={path}
        aria-label={label}
        className="min-w-0 flex-1 cursor-text select-text truncate"
      />
      <Button onClick={onOpen} disabled={disabled}>
        {t("common.open")}
      </Button>
    </div>
  );
};
