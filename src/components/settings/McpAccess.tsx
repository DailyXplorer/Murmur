import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { MCP_CLIENTS, mcpSetup, type McpClient } from "@/lib/mcp";
import { useSettings } from "../../hooks/useSettings";
import { Button } from "../ui/Button";
import { SettingContainer } from "../ui/SettingContainer";
import { Tabs, panelId, tabId } from "../ui/Tabs";
import { ToggleSwitch } from "../ui/ToggleSwitch";

const COPIED_MS = 2000;
const ID_PREFIX = "mcp-client";

/** Lets AI apps read meetings through `murmur mcp`, off until turned on, and
 * shows what to paste into the app. */
export const McpAccess: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const enabled = getSetting("mcp_enabled") ?? false;
  const [command, setCommand] = useState<string | null>(null);
  const [client, setClient] = useState<McpClient>("claude_code");
  const [copied, setCopied] = useState(false);
  const copiedTimer = useRef<number | null>(null);

  useEffect(() => {
    let current = true;
    void commands.getMcpSetup().then((result) => {
      if (current && result.status === "ok") setCommand(result.data.command);
    });
    return () => {
      current = false;
    };
  }, []);

  useEffect(
    () => () => {
      if (copiedTimer.current !== null)
        window.clearTimeout(copiedTimer.current);
    },
    [],
  );

  const setup = command === null ? "" : mcpSetup(client, command);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(setup);
    } catch (error) {
      console.error("Failed to copy to clipboard:", error);
      return;
    }
    setCopied(true);
    if (copiedTimer.current !== null) window.clearTimeout(copiedTimer.current);
    copiedTimer.current = window.setTimeout(() => setCopied(false), COPIED_MS);
  };

  return (
    <>
      <ToggleSwitch
        checked={enabled}
        onChange={(value) => updateSetting("mcp_enabled", value)}
        isUpdating={isUpdating("mcp_enabled")}
        label={t("settings.advanced.mcp.label")}
        description={t("settings.advanced.mcp.description")}
        grouped={true}
      />
      {enabled && (
        <ToggleSwitch
          checked={getSetting("mcp_include_dictations") ?? false}
          onChange={(value) => updateSetting("mcp_include_dictations", value)}
          isUpdating={isUpdating("mcp_include_dictations")}
          label={t("settings.advanced.mcp.dictations")}
          description={t("settings.advanced.mcp.dictationsDescription")}
          grouped={true}
        />
      )}
      {enabled && command !== null && (
        <SettingContainer
          title={t("settings.advanced.mcp.setupTitle")}
          description={t("settings.advanced.mcp.setupDescription")}
          grouped={true}
          layout="stacked"
        >
          <div className="space-y-3">
            <Tabs
              tabs={MCP_CLIENTS.map((id) => ({
                id,
                label: t(`settings.advanced.mcp.clients.${id}`),
              }))}
              selected={client}
              onSelect={setClient}
              label={t("settings.advanced.mcp.clientsLabel")}
              idPrefix={ID_PREFIX}
            />
            <div
              role="tabpanel"
              id={panelId(ID_PREFIX, client)}
              aria-labelledby={tabId(ID_PREFIX, client)}
              className="space-y-3"
            >
              <pre
                data-testid="mcp-setup"
                className="select-text overflow-x-auto whitespace-pre rounded-md border border-mid-gray/20 bg-mid-gray/10 p-3 font-mono text-xs"
              >
                {setup}
              </pre>
              <div className="flex justify-end">
                <Button onClick={() => void copy()}>
                  {copied
                    ? t("settings.advanced.mcp.copied")
                    : t("settings.advanced.mcp.copyConfig")}
                </Button>
              </div>
            </div>
          </div>
        </SettingContainer>
      )}
    </>
  );
};
