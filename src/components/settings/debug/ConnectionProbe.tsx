import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type ProbeResult } from "@/bindings";
import { Button } from "../../ui/Button";
import { SettingContainer } from "../../ui/SettingContainer";

interface ConnectionProbeProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

export const ConnectionProbe: React.FC<ConnectionProbeProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const [results, setResults] = useState<ProbeResult[]>([]);
  const [isRunning, setIsRunning] = useState(false);

  const run = async () => {
    setIsRunning(true);
    try {
      setResults(await commands.testCodexConnection());
    } finally {
      setIsRunning(false);
    }
  };

  return (
    <SettingContainer
      title={t("settings.debug.connectionProbe.title")}
      description={t("settings.debug.connectionProbe.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
      layout="stacked"
    >
      <Button onClick={run} disabled={isRunning}>
        {isRunning
          ? t("settings.debug.connectionProbe.running")
          : t("settings.debug.connectionProbe.button")}
      </Button>
      {results.length > 0 && (
        <ul className="mt-2 space-y-1 font-mono text-xs">
          {results.map((result) => (
            <li
              key={result.name}
              className={result.passed ? "text-green-500" : "text-red-400"}
            >
              {t("settings.debug.connectionProbe.result", {
                name: result.name,
                outcome: result.outcome,
              })}
            </li>
          ))}
        </ul>
      )}
    </SettingContainer>
  );
};
