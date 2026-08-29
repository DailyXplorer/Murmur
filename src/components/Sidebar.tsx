import React from "react";
import { useTranslation } from "react-i18next";
import { ClockCounterClockwiseIcon } from "@phosphor-icons/react/dist/csr/ClockCounterClockwise";
import { CloudIcon } from "@phosphor-icons/react/dist/csr/Cloud";
import { FlaskIcon } from "@phosphor-icons/react/dist/csr/Flask";
import { GearSixIcon } from "@phosphor-icons/react/dist/csr/GearSix";
import { InfoIcon } from "@phosphor-icons/react/dist/csr/Info";
import { MicrophoneIcon } from "@phosphor-icons/react/dist/csr/Microphone";
import type { Icon } from "@phosphor-icons/react/dist/lib/types";
import type { AppSettings } from "@/bindings";
import MurmurTextLogo from "./icons/MurmurTextLogo";
import { useSettings } from "../hooks/useSettings";
import {
  GeneralSettings,
  AdvancedSettings,
  HistorySettings,
  DebugSettings,
  AboutSettings,
  TranscriptionSettings,
} from "./settings";

interface SectionConfig {
  labelKey: string;
  icon: Icon;
  component: React.ComponentType;
  enabled: (settings: AppSettings | null) => boolean;
}

export const SECTIONS_CONFIG = {
  general: {
    labelKey: "sidebar.general",
    icon: MicrophoneIcon,
    component: GeneralSettings,
    enabled: () => true,
  },
  transcription: {
    labelKey: "sidebar.transcription",
    icon: CloudIcon,
    component: TranscriptionSettings,
    enabled: () => true,
  },
  history: {
    labelKey: "sidebar.history",
    icon: ClockCounterClockwiseIcon,
    component: HistorySettings,
    enabled: () => true,
  },
  advanced: {
    labelKey: "sidebar.advanced",
    icon: GearSixIcon,
    component: AdvancedSettings,
    enabled: () => true,
  },
  debug: {
    labelKey: "sidebar.debug",
    icon: FlaskIcon,
    component: DebugSettings,
    enabled: (settings) => settings?.debug_mode ?? false,
  },
  about: {
    labelKey: "sidebar.about",
    icon: InfoIcon,
    component: AboutSettings,
    enabled: () => true,
  },
} as const satisfies Record<string, SectionConfig>;

export type SidebarSection = keyof typeof SECTIONS_CONFIG;

const SECTION_ORDER: readonly SidebarSection[] = [
  "general",
  "transcription",
  "history",
  "advanced",
  "debug",
  "about",
];

interface SidebarProps {
  activeSection: SidebarSection;
  onSectionChange: (section: SidebarSection) => void;
}

export const Sidebar: React.FC<SidebarProps> = ({
  activeSection,
  onSectionChange,
}) => {
  const { t } = useTranslation();
  const { settings } = useSettings();

  const availableSections = SECTION_ORDER.filter((sectionId) =>
    SECTIONS_CONFIG[sectionId].enabled(settings),
  );

  return (
    <nav
      aria-label={t("tray.settings")}
      className="flex h-full w-40 flex-col items-center border-e border-mid-gray/20 px-2"
    >
      <MurmurTextLogo width={120} className="m-4" />
      <div className="flex w-full flex-col items-center gap-1 border-t border-mid-gray/20 pt-2">
        {availableSections.map((sectionId) => {
          const section = SECTIONS_CONFIG[sectionId];
          const Icon = section.icon;
          const isActive = activeSection === sectionId;

          return (
            <button
              key={sectionId}
              type="button"
              aria-current={isActive ? "page" : undefined}
              className={`flex min-h-10 w-full cursor-pointer appearance-none items-center gap-2 rounded-lg border-0 p-2 text-start transition-[color,background-color,opacity,transform] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary focus-visible:ring-offset-2 focus-visible:ring-offset-background active:scale-[0.96] ${
                isActive
                  ? "bg-background-ui text-on-accent hover:bg-background-ui-hover active:bg-background-ui-active"
                  : "bg-transparent opacity-85 hover:bg-mid-gray/20 hover:opacity-100"
              }`}
              onClick={() => onSectionChange(sectionId)}
            >
              <Icon size={20} className="shrink-0" aria-hidden="true" />
              <span
                className="min-w-0 truncate text-sm font-medium"
                title={t(section.labelKey)}
              >
                {t(section.labelKey)}
              </span>
            </button>
          );
        })}
      </div>
    </nav>
  );
};
