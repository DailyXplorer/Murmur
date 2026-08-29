import React, { useState } from "react";
import { XIcon } from "@phosphor-icons/react/dist/csr/X";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useSettings } from "../../hooks/useSettings";
import { Input } from "../ui/Input";
import { Button } from "../ui/Button";
import { SettingContainer } from "../ui/SettingContainer";
import { SettingControlGroup } from "../ui/SettingControlGroup";

interface CustomWordsProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

const normalizeCustomWord = (word: string) =>
  word
    .replace(/[<>"']/g, "")
    .replace(/\s+/g, " ")
    .trim();

export const CustomWords: React.FC<CustomWordsProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const [newWord, setNewWord] = useState("");
    const customWords = getSetting("custom_words") || [];
    const normalizedWord = normalizeCustomWord(newWord);

    const handleAddWord = () => {
      if (normalizedWord && normalizedWord.length <= 50) {
        if (customWords.includes(normalizedWord)) {
          toast.error(
            t("settings.advanced.customWords.duplicate", {
              word: normalizedWord,
            }),
          );
          return;
        }
        updateSetting("custom_words", [...customWords, normalizedWord]);
        setNewWord("");
      }
    };

    const handleRemoveWord = (wordToRemove: string) => {
      updateSetting(
        "custom_words",
        customWords.filter((word) => word !== wordToRemove),
      );
    };

    const handleKeyPress = (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        handleAddWord();
      }
    };

    return (
      <>
        <SettingContainer
          title={t("settings.advanced.customWords.title")}
          description={t("settings.advanced.customWords.description")}
          descriptionMode={descriptionMode}
          grouped={grouped}
          controlSizing="content"
        >
          <SettingControlGroup
            primary={
              <Input
                type="text"
                className="w-full"
                value={newWord}
                onChange={(e) => setNewWord(e.target.value)}
                onKeyDown={handleKeyPress}
                placeholder={t("settings.advanced.customWords.placeholder")}
                variant="compact"
                disabled={isUpdating("custom_words")}
              />
            }
            action={
              <Button
                onClick={handleAddWord}
                disabled={
                  !normalizedWord ||
                  normalizedWord.length > 50 ||
                  isUpdating("custom_words")
                }
                variant="primary"
                size="md"
              >
                {t("settings.advanced.customWords.add")}
              </Button>
            }
          />
        </SettingContainer>
        {customWords.length > 0 && (
          <div
            className={`flex justify-end px-4 p-2 ${grouped ? "" : "rounded-lg border border-mid-gray/20"}`}
          >
            <div className="flex w-[var(--settings-control-rail-width)] flex-wrap gap-1">
              {customWords.map((word) => (
                <Button
                  key={word}
                  onClick={() => handleRemoveWord(word)}
                  disabled={isUpdating("custom_words")}
                  variant="secondary"
                  size="sm"
                  className="inline-flex max-w-full min-w-0 cursor-pointer items-center gap-1"
                  aria-label={t("settings.advanced.customWords.remove", {
                    word,
                  })}
                  title={word}
                >
                  <span className="min-w-0 truncate">{word}</span>
                  <XIcon className="shrink-0" size={12} />
                </Button>
              ))}
            </div>
          </div>
        )}
      </>
    );
  },
);
