import { useCallback, useRef, useState } from "react";
import type { CodexAuthStatus, GeminiStatus } from "@/bindings";
import { commands } from "@/bindings";
import { isWindows } from "@/lib/platform";

export type ProviderStatus =
  | { kind: "checking" }
  | { kind: "configured" }
  | {
      kind: "unavailable";
      reason: "notInstalled" | "notSignedIn" | "sessionExpired";
    }
  | { kind: "error" };

export interface ProviderStatuses {
  codex: ProviderStatus;
  gemini: ProviderStatus;
}

export const CHECKING_PROVIDER_STATUSES: ProviderStatuses = {
  codex: { kind: "checking" },
  gemini: { kind: "checking" },
};

/** Antigravity transcription is macOS only. */
const WINDOWS_GEMINI_STATUS: GeminiStatus = {
  installed: false,
  signed_in: false,
};

const codexStatusFrom = (
  result: PromiseSettledResult<CodexAuthStatus>,
): ProviderStatus => {
  if (result.status === "rejected") return { kind: "error" };
  if (result.value.signed_in) return { kind: "configured" };
  return {
    kind: "unavailable",
    reason: result.value.expired ? "sessionExpired" : "notSignedIn",
  };
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
      isWindows ? WINDOWS_GEMINI_STATUS : commands.getGeminiStatus(),
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

export const statusTranslationKey = (status: ProviderStatus): string => {
  switch (status.kind) {
    case "checking":
      return "settings.transcription.checking";
    case "configured":
      return "onboarding.provider.configured";
    case "unavailable":
      switch (status.reason) {
        case "notInstalled":
          return "settings.transcription.notInstalled";
        case "sessionExpired":
          return "settings.transcription.sessionExpired";
        case "notSignedIn":
          return "onboarding.providerLabels.unavailable";
        default: {
          const _exhaustive: never = status.reason;
          return _exhaustive;
        }
      }
    case "error":
      return "settings.transcription.statusUnavailable";
    default: {
      const _exhaustive: never = status;
      return _exhaustive;
    }
  }
};
