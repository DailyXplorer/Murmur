import React, { useState } from "react";
import ReactDOM from "react-dom/client";
import { XIcon } from "@phosphor-icons/react/dist/csr/X";
import { Button } from "../../src/components/ui/Button";
import { Dropdown } from "../../src/components/ui/Dropdown";
import { Input } from "../../src/components/ui/Input";
import { PathDisplay } from "../../src/components/ui/PathDisplay";
import { ResetButton } from "../../src/components/ui/ResetButton";
import { SettingContainer } from "../../src/components/ui/SettingContainer";
import { SettingControlGroup } from "../../src/components/ui/SettingControlGroup";
import { ToggleSwitch } from "../../src/components/ui/ToggleSwitch";
import "../../src/App.css";

const OPTIONS = [
  { value: "first", label: "First option" },
  { value: "second", label: "Second option" },
];

const LONG_CUSTOM_WORD = "W".repeat(50);

const SettingsControlWidthsFixture: React.FC = () => {
  const [selectedValue, setSelectedValue] = useState<string | null>("first");
  const [isToggleChecked, setIsToggleChecked] = useState(false);

  return (
    <main className="min-h-screen bg-background px-4 py-10 text-text">
      <section
        data-testid="settings-control-widths"
        className="mx-auto flex w-full max-w-[768px] flex-col gap-2"
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
          description="The primary dropdown keeps the full rail"
          controlSizing="content"
        >
          <SettingControlGroup
            primary={
              <div data-testid="dropdown-reset-primary">
                <Dropdown
                  options={OPTIONS}
                  selectedValue={selectedValue}
                  onSelect={setSelectedValue}
                />
              </div>
            }
            action={
              <ResetButton
                ariaLabel="Reset dropdown"
                onClick={() => undefined}
              />
            }
          />
        </SettingContainer>

        <SettingContainer
          title="Language-like picker"
          description="A bespoke picker keeps the full rail"
          controlSizing="content"
        >
          <SettingControlGroup
            primary={
              <button
                data-testid="language-picker-primary"
                className="min-h-10 w-full rounded-md border border-mid-gray/80 bg-mid-gray/10 px-2 py-1 text-start text-sm"
                type="button"
              >
                English (United States)
              </button>
            }
            action={
              <ResetButton
                ariaLabel="Reset language"
                onClick={() => undefined}
              />
            }
          />
        </SettingContainer>

        <ToggleSwitch
          checked={isToggleChecked}
          description="The toggle remains compact at the rail edge"
          label="Recording indicator"
          onChange={setIsToggleChecked}
        />

        <SettingContainer
          title="Shortcut-like control"
          description="A long shortcut keeps the full rail"
          controlSizing="content"
        >
          <SettingControlGroup
            primary={
              <div
                className="w-full cursor-pointer truncate rounded-md border border-mid-gray/80 bg-mid-gray/10 px-2 py-1 text-start text-sm font-normal hover:border-logo-primary hover:bg-logo-primary/10"
                data-testid="shortcut-primary"
              >
                Command Shift Option K
              </div>
            }
            action={
              <ResetButton
                ariaLabel="Reset shortcut"
                onClick={() => undefined}
              />
            }
          />
        </SettingContainer>

        <SettingContainer
          title="History limit"
          description="Compact number input and label"
        >
          <div
            data-testid="history-limit"
            className="flex items-center justify-end gap-2"
          >
            <Input className="w-20" type="number" value="50" readOnly />
            <span className="text-sm">entries</span>
          </div>
        </SettingContainer>

        <SettingContainer
          title="Custom words"
          description="The input keeps the full rail beside Add"
          controlSizing="content"
        >
          <SettingControlGroup
            primary={
              <Input
                data-testid="custom-words-primary"
                className="w-full"
                placeholder="Add a custom word"
                type="text"
              />
            }
            action={<Button size="md">Add</Button>}
          />
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
            <Button
              className="inline-flex max-w-full min-w-0 cursor-pointer items-center gap-1"
              data-testid="long-custom-word"
              size="sm"
              variant="secondary"
            >
              <span className="min-w-0 truncate">{LONG_CUSTOM_WORD}</span>
              <XIcon className="shrink-0" size={12} />
            </Button>
          </div>
        </div>

        <SettingContainer
          title="Gemini status"
          description="Status and action retain the standard slot"
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
          title="App data directory"
          description="The path uses the complete stacked width"
        >
          <div data-testid="path-display">
            <PathDisplay
              path="/Users/example/Library/Application Support/com.dailyxplorer.murmur"
              onOpen={() => undefined}
            />
          </div>
        </SettingContainer>
      </section>
    </main>
  );
};

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <SettingsControlWidthsFixture />,
);
