import { useCallback, useRef, useState } from "react";
import type { CodexAuthStatus, GeminiStatus } from "@/bindings";
import { commands } from "@/bindings";

export type ProviderStatus =
  | { kind: "checking" }
  | { kind: "configured" }
  | { kind: "unavailable"; reason: "notInstalled" | "notSignedIn" }
  | { kind: "error" };

export interface ProviderStatuses {
  codex: ProviderStatus;
  gemini: ProviderStatus;
}

export const CHECKING_PROVIDER_STATUSES: ProviderStatuses = {
  codex: { kind: "checking" },
  gemini: { kind: "checking" },
};

const codexStatusFrom = (
  result: PromiseSettledResult<CodexAuthStatus>,
): ProviderStatus => {
  if (result.status === "rejected") return { kind: "error" };
  return result.value.signed_in
    ? { kind: "configured" }
    : { kind: "unavailable", reason: "notSignedIn" };
};

const geminiStatusFrom = (
  result: PromiseSettledResult<GeminiStatus>,
): ProviderStatus => {
  if (result.status === "rejected") return { kind: "error" };
  if (!result.value.installed) {
    return { kind: "unavailable", reason: "notInstalled" };
  }
  return result.value.signed_in
    ? { kind: "configured" }
    : { kind: "unavailable", reason: "notSignedIn" };
};

/**
 * Reads local provider configuration without claiming a live connection.
 * A request sequence keeps delayed focus checks from replacing newer results.
 */
export const useProviderStatuses = () => {
  const [statuses, setStatuses] = useState<ProviderStatuses>(
    CHECKING_PROVIDER_STATUSES,
  );
  const requestSequence = useRef(0);

  const refreshStatuses = useCallback(async () => {
    const sequence = ++requestSequence.current;
    setStatuses(CHECKING_PROVIDER_STATUSES);
    const [codex, gemini] = await Promise.allSettled([
      commands.getCodexAuthStatus(),
      commands.getGeminiStatus(),
    ]);

    if (sequence !== requestSequence.current) return;

    setStatuses({
      codex: codexStatusFrom(codex),
      gemini: geminiStatusFrom(gemini),
    });
  }, []);

  const invalidatePendingRefreshes = useCallback(() => {
    requestSequence.current += 1;
  }, []);

  return { invalidatePendingRefreshes, refreshStatuses, statuses };
};

export const isConfiguredProvider = (status: ProviderStatus): boolean =>
  status.kind === "configured";
