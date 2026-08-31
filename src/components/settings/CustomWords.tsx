import React, { useState } from "react";
import { XIcon } from "@phosphor-icons/react/dist/csr/X";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useSettings } from "../../hooks/useSettings";
import { Input } from "../ui/Input";
import { Button } from "../ui/Button";
import { SettingContainer } from "../ui/SettingContainer";

interface CustomWordsProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

const MAX_CUSTOM_WORDS = 200;

const normalizeCustomWord = (word: string) => word.replace(/\s+/g, " ").trim();

export const CustomWords: React.FC<CustomWordsProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();
    const [newWord, setNewWord] = useState("");
    const customWords = getSetting("custom_words") || [];
    const normalizedWord = normalizeCustomWord(newWord);
    const normalizedWordLength = Array.from(normalizedWord).length;

    const handleAddWord = async () => {
      if (normalizedWord && normalizedWordLength <= 50) {
        if (customWords.includes(normalizedWord)) {
          toast.error(
            t("settings.advanced.customWords.duplicate", {
              word: normalizedWord,
            }),
          );
          return;
        }
        if (
          customWords.length < MAX_CUSTOM_WORDS &&
          (await updateSetting("custom_words", [
            ...customWords,
            normalizedWord,
          ]))
        ) {
          setNewWord("");
        }
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
        void handleAddWord();
      }
    };

    return (
      <>
        <SettingContainer
          title={t("settings.advanced.customWords.title")}
          description={t("settings.advanced.customWords.description")}
          descriptionMode={descriptionMode}
          grouped={grouped}
        >
          <div className="flex min-w-0 items-center gap-2">
            <Input
              type="text"
              className="min-w-0 flex-1"
              value={newWord}
              onChange={(e) => setNewWord(e.target.value)}
              onKeyDown={handleKeyPress}
              placeholder={t("settings.advanced.customWords.placeholder")}
              variant="compact"
              disabled={isUpdating("custom_words")}
            />
            <Button
              onClick={() => void handleAddWord()}
              disabled={
                !normalizedWord ||
                normalizedWordLength > 50 ||
                customWords.length >= MAX_CUSTOM_WORDS ||
                isUpdating("custom_words")
              }
              variant="primary"
              size="md"
              className="shrink-0"
            >
              {t("settings.advanced.customWords.add")}
            </Button>
          </div>
        </SettingContainer>
        {customWords.length > 0 && (
          <div
            className={`px-4 py-2 ${grouped ? "" : "rounded-lg border border-mid-gray/20"}`}
          >
            <div className="flex flex-wrap justify-start gap-1">
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
