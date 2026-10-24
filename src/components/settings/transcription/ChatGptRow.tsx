import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type Engine } from "@/bindings";
import {
  type ProviderStatus,
  signInErrorKey,
  statusTranslationKey,
  useChatGptSignIn,
} from "@/components/transcription/providerStatus";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { SettingContainer } from "../../ui/SettingContainer";

/** Asks before signing out, since the session is the one Codex uses too. */
const SignOutDialog: React.FC<{
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: () => void;
}> = ({ open, onOpenChange, onConfirm }) => {
  const { t } = useTranslation();
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("settings.transcription.chatgpt.signOutDialog.title")}
      closeLabel={t("common.close")}
      footer={
        <>
          <Button onClick={() => onOpenChange(false)}>
            {t("meetings.cancel")}
          </Button>
          <Button
            onClick={() => {
              onOpenChange(false);
              onConfirm();
            }}
          >
            {t("settings.transcription.chatgpt.signOut")}
          </Button>
        </>
      }
    >
      <p className="text-sm text-text/80">
        {t("settings.transcription.chatgpt.signOutDialog.body")}
      </p>
    </Dialog>
  );
};

/** The ChatGPT session: its state, and the way to sign in or out. The
 * account's e-mail is never shown. */
export const ChatGptRow: React.FC<{
  status: ProviderStatus;
  refreshStatus: () => Promise<void>;
  engine: Engine;
}> = ({ status, refreshStatus, engine }) => {
  const { t } = useTranslation();
  const [confirming, setConfirming] = useState(false);
  // The last finished check, so a refresh on focus does not swap the button.
  const [checked, setChecked] = useState<ProviderStatus | null>(null);
  if (status.kind !== "checking" && status !== checked) setChecked(status);

  const { cancel, signIn, start } = useChatGptSignIn(refreshStatus);

  const signOut = async () => {
    try {
      const result = await commands.signOutOfChatgpt();
      if (result.status === "error") throw new Error(result.error);
    } catch (error) {
      console.warn("Failed to sign out of ChatGPT:", error);
      toast.error(t("settings.transcription.chatgpt.signOutFailed"));
    }
    await refreshStatus();
  };

  const signedIn = checked?.kind === "configured";
  const description =
    signIn.phase === "waiting"
      ? t("settings.transcription.chatgpt.waiting")
      : signIn.phase === "failed" && !signedIn
        ? t(signInErrorKey(signIn.error))
        : engine === "local" &&
            checked?.kind === "unavailable" &&
            checked.reason === "notSignedIn"
          ? t("settings.transcription.chatgpt.optional")
          : t(statusTranslationKey(checked ?? status));

  return (
    <SettingContainer
      title={t("settings.transcription.chatgpt.title")}
      description={description}
      descriptionMode="inline"
      grouped={true}
    >
      {signIn.phase === "waiting" ? (
        <Button onClick={cancel}>{t("meetings.cancel")}</Button>
      ) : signedIn ? (
        <Button onClick={() => setConfirming(true)}>
          {t("settings.transcription.chatgpt.signOut")}
        </Button>
      ) : (
        <Button onClick={() => void start()} disabled={checked === null}>
          {t("settings.transcription.chatgpt.signIn")}
        </Button>
      )}
      <SignOutDialog
        open={confirming}
        onOpenChange={setConfirming}
        onConfirm={() => void signOut()}
      />
    </SettingContainer>
  );
};
