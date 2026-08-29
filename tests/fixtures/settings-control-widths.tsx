import React, { useState } from "react";
import ReactDOM from "react-dom/client";
import { Button } from "../../src/components/ui/Button";
import { Dropdown } from "../../src/components/ui/Dropdown";
import { Input } from "../../src/components/ui/Input";
import { ResetButton } from "../../src/components/ui/ResetButton";
import { SettingContainer } from "../../src/components/ui/SettingContainer";
import { ToggleSwitch } from "../../src/components/ui/ToggleSwitch";
import "../../src/App.css";

const OPTIONS = [
  { value: "first", label: "First option" },
  { value: "second", label: "Second option" },
];

const SettingsControlWidthsFixture: React.FC = () => {
  const [selectedValue, setSelectedValue] = useState<string | null>("first");
  const [isToggleChecked, setIsToggleChecked] = useState(false);

  return (
    <main className="min-h-screen bg-background px-8 py-10 text-text">
      <section
        data-testid="settings-control-widths"
        className="mx-auto flex w-[768px] flex-col gap-2"
      >
        <SettingContainer
          title="Direct dropdown"
          description="A direct full-rail dropdown"
        >
          <div data-testid="direct-dropdown">
            <Dropdown
              options={OPTIONS}
              selectedValue={selectedValue}
              onSelect={setSelectedValue}
            />
          </div>
        </SettingContainer>

        <SettingContainer
          title="Dropdown with reset"
          description="The reset target stays compact"
        >
          <div
            data-testid="dropdown-reset"
            className="flex min-w-0 items-center gap-1"
          >
            <Dropdown
              className="min-w-0 flex-1"
              options={OPTIONS}
              selectedValue={selectedValue}
              onSelect={setSelectedValue}
            />
            <ResetButton ariaLabel="Reset dropdown" onClick={() => undefined} />
          </div>
        </SettingContainer>

        <SettingContainer
          title="Language-like picker"
          description="A bespoke picker shares the rail with reset"
        >
          <div
            data-testid="language-picker"
            className="flex min-w-0 items-center gap-1"
          >
            <button
              className="min-w-0 flex-1 rounded-md border border-mid-gray/80 bg-mid-gray/10 px-2 py-1 text-start text-sm"
              type="button"
            >
              English (United States)
            </button>
            <ResetButton ariaLabel="Reset language" onClick={() => undefined} />
          </div>
        </SettingContainer>

        <ToggleSwitch
          checked={isToggleChecked}
          description="The toggle remains a 44px target at the rail edge"
          label="Recording indicator"
          onChange={setIsToggleChecked}
        />

        <SettingContainer
          title="Shortcut-like control"
          description="A long shortcut chip still leaves reset intrinsic"
        >
          <div
            data-testid="shortcut-control"
            className="flex min-w-0 items-center gap-1"
          >
            <button
              className="min-w-0 truncate rounded-md border border-mid-gray/80 bg-mid-gray/10 px-2 py-1 text-sm"
              type="button"
            >
              Command Shift Control Option Transcription
            </button>
            <ResetButton ariaLabel="Reset shortcut" onClick={() => undefined} />
          </div>
        </SettingContainer>

        <SettingContainer
          title="History limit"
          description="Compact number input and label"
        >
          <div data-testid="history-limit" className="flex items-center gap-2">
            <Input className="w-20" type="number" value="50" readOnly />
            <span className="text-sm">entries</span>
          </div>
        </SettingContainer>

        <SettingContainer
          title="Custom words"
          description="Input and add action stay within the rail"
        >
          <div
            data-testid="custom-words"
            className="flex min-w-0 items-center gap-2"
          >
            <Input
              className="min-w-0 flex-1"
              placeholder="Add a custom word"
              type="text"
            />
            <Button className="shrink-0" size="md">
              Add
            </Button>
          </div>
        </SettingContainer>
        <div className="flex justify-end rounded-lg border border-mid-gray/20 px-4 p-2">
          <div
            data-testid="custom-word-chips"
            className="flex w-[var(--settings-control-rail-width)] flex-wrap gap-1"
          >
            <Button size="sm" variant="secondary">
              Acme
            </Button>
            <Button size="sm" variant="secondary">
              Long custom transcription term
            </Button>
          </div>
        </div>

        <SettingContainer
          title="Gemini status"
          description="Status and action share the same rail"
        >
          <div
            data-testid="gemini-status"
            className="flex min-w-0 items-center gap-2"
          >
            <span className="min-w-0 flex-1 truncate text-sm text-text/80">
              Gemini transcription service is not installed
            </span>
            <Button className="shrink-0" size="sm" variant="secondary">
              Install
            </Button>
          </div>
        </SettingContainer>

        <SettingContainer
          layout="stacked"
          title="Stacked content"
          description="This content intentionally remains full width"
        >
          <div
            data-testid="stacked-control"
            className="h-10 rounded-md border border-mid-gray/80 bg-mid-gray/10"
          />
        </SettingContainer>
      </section>
    </main>
  );
};

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <SettingsControlWidthsFixture />,
);
