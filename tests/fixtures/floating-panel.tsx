import React, { useCallback, useRef, useState } from "react";
import ReactDOM from "react-dom/client";
import { Dialog } from "../../src/components/ui/Dialog";
import { Dropdown } from "../../src/components/ui/Dropdown";
import { FloatingPanel } from "../../src/components/ui/FloatingPanel";
import "../../src/App.css";

const searchParams = new URLSearchParams(window.location.search);
const requestedOptionCount = Number(searchParams.get("options") ?? 12);
const optionCount = Number.isFinite(requestedOptionCount)
  ? requestedOptionCount
  : 12;
const options = Array.from({ length: optionCount }, (_, index) => ({
  value: `option-${index + 1}`,
  label: `Option ${index + 1}`,
}));

/** Fixture with a search field that can swallow Escape before the panel dismisses. */
const SearchPanelFixture: React.FC = () => {
  const [isOpen, setIsOpen] = useState(false);
  const [lastInputKey, setLastInputKey] = useState("");
  const [searchValue, setSearchValue] = useState("");
  const triggerRef = useRef<HTMLButtonElement>(null);
  const handleDismiss = useCallback(() => setIsOpen(false), []);

  return (
    <div className="flex h-screen items-center justify-center bg-background text-text">
      <button ref={triggerRef} type="button" onClick={() => setIsOpen(true)}>
        Open searchable panel
      </button>
      <output>Last input key: {lastInputKey}</output>
      <FloatingPanel
        open={isOpen}
        anchorRef={triggerRef}
        onDismiss={handleDismiss}
      >
        <input
          aria-label="Search options"
          autoFocus
          value={searchValue}
          onChange={(event) => setSearchValue(event.target.value)}
          onKeyDown={(event) => {
            setLastInputKey(event.key);
            if (event.key === "Escape") event.preventDefault();
          }}
        />
        <button type="button" role="option">
          Search result
        </button>
      </FloatingPanel>
    </div>
  );
};

/** Fixture with two stacked panels used to assert topmost Escape dismissal. */
const MultiplePanelsFixture: React.FC = () => {
  const [isFirstOpen, setIsFirstOpen] = useState(true);
  const [isSecondOpen, setIsSecondOpen] = useState(true);
  const firstTriggerRef = useRef<HTMLButtonElement>(null);
  const secondTriggerRef = useRef<HTMLButtonElement>(null);
  const dismissFirst = useCallback(() => setIsFirstOpen(false), []);
  const dismissSecond = useCallback(() => setIsSecondOpen(false), []);

  return (
    <div className="flex h-screen items-center justify-center gap-4 bg-background text-text">
      <button ref={firstTriggerRef} type="button">
        First trigger
      </button>
      <button ref={secondTriggerRef} type="button">
        Second trigger
      </button>
      <FloatingPanel
        open={isFirstOpen}
        anchorRef={firstTriggerRef}
        onDismiss={dismissFirst}
      >
        First panel
      </FloatingPanel>
      <FloatingPanel
        open={isSecondOpen}
        anchorRef={secondTriggerRef}
        onDismiss={dismissSecond}
      >
        Second panel
      </FloatingPanel>
    </div>
  );
};

/** Dialog with a nested portaled panel plus outside controls for Tab trapping. */
const DialogLayeringFixture: React.FC = () => {
  const [isDialogOpen, setIsDialogOpen] = useState(true);
  const [isOutsidePanelOpen, setIsOutsidePanelOpen] = useState(true);
  const [isNestedPanelOpen, setIsNestedPanelOpen] = useState(false);
  const outsideTriggerRef = useRef<HTMLButtonElement>(null);
  const nestedTriggerRef = useRef<HTMLButtonElement>(null);
  const dismissOutsidePanel = useCallback(
    () => setIsOutsidePanelOpen(false),
    [],
  );
  const dismissNestedPanel = useCallback(() => setIsNestedPanelOpen(false), []);

  return (
    <div className="h-screen bg-background text-text">
      <button
        ref={outsideTriggerRef}
        type="button"
        className="fixed left-4 top-4"
      >
        Outside trigger
      </button>
      <button
        data-testid="outside-dialog-control"
        type="button"
        className="fixed left-4 top-16"
      >
        Outside dialog control
      </button>
      <FloatingPanel
        open={isOutsidePanelOpen}
        anchorRef={outsideTriggerRef}
        onDismiss={dismissOutsidePanel}
        className="bg-background"
      >
        <div data-testid="outside-dialog-panel">Outside dialog panel</div>
      </FloatingPanel>
      <Dialog
        open={isDialogOpen}
        title="Layering dialog"
        closeLabel="Close dialog"
        onOpenChange={setIsDialogOpen}
        initialFocusRef={nestedTriggerRef}
        contentFades={false}
      >
        <button data-testid="inside-dialog-before" type="button">
          Inside before
        </button>
        <button
          ref={nestedTriggerRef}
          type="button"
          onClick={() => setIsNestedPanelOpen(true)}
        >
          Nested trigger
        </button>
        <button data-testid="inside-dialog-after" type="button">
          Inside after
        </button>
        <FloatingPanel
          open={isNestedPanelOpen}
          anchorRef={nestedTriggerRef}
          onDismiss={dismissNestedPanel}
          className="bg-background"
          focusFirstOptionOnOpen
        >
          <div data-testid="nested-dialog-panel">
            <button type="button" role="option">
              Nested option
            </button>
          </div>
        </FloatingPanel>
      </Dialog>
    </div>
  );
};

/** Dropdown whose trigger disables after selection until a timer re-enables it. */
const DisabledTriggerFixture: React.FC = () => {
  const [selectedValue, setSelectedValue] = useState<string | null>(null);
  const [isUpdating, setIsUpdating] = useState(false);

  /** Selects a value then briefly disables the trigger to test delayed restore. */
  const handleSelect = (value: string) => {
    setSelectedValue(value);
    setIsUpdating(true);
    window.setTimeout(() => setIsUpdating(false), 120);
  };

  return (
    <div className="flex h-screen items-center justify-center bg-background text-text">
      <Dropdown
        className="w-64"
        options={options.slice(0, 3)}
        selectedValue={selectedValue}
        onSelect={handleSelect}
        disabled={isUpdating}
      />
    </div>
  );
};

/** Dropdown inside a scrollable ancestor used to assert clip-based dismissal. */
const ScrollContainerFixture: React.FC = () => {
  const [selectedValue, setSelectedValue] = useState<string | null>(null);

  return (
    <div className="flex h-screen items-center justify-center bg-background text-text">
      <div
        data-testid="scroll-container"
        className="h-32 w-80 overflow-y-auto border border-mid-gray/20"
      >
        <div className="p-2">
          <Dropdown
            className="w-64"
            options={options.slice(0, 3)}
            selectedValue={selectedValue}
            onSelect={setSelectedValue}
          />
        </div>
        <div className="h-80" />
      </div>
    </div>
  );
};

/** Default clipped dropdown with neighboring form controls for Tab order. */
const FloatingPanelFixture: React.FC = () => {
  const [selectedValue, setSelectedValue] = useState<string | null>(null);

  return (
    <div className="h-screen bg-background text-text">
      <div data-testid="clipping-container" className="h-72 overflow-hidden">
        <div className="flex h-full items-end justify-center gap-2 pb-2">
          <button data-testid="before-control" type="button">
            Before control
          </button>
          <Dropdown
            className="w-64"
            options={options}
            selectedValue={selectedValue}
            onSelect={setSelectedValue}
          />
        </div>
      </div>
      <footer
        data-testid="footer"
        className="relative z-[1050] h-24 border-t border-mid-gray/20 bg-background"
      >
        <button data-testid="outside-control" type="button">
          Outside control
        </button>
      </footer>
    </div>
  );
};

const fixture = searchParams.has("multiple") ? (
  <MultiplePanelsFixture />
) : searchParams.has("dialog-layers") ? (
  <DialogLayeringFixture />
) : searchParams.has("disabled-trigger") ? (
  <DisabledTriggerFixture />
) : searchParams.has("scroll-container") ? (
  <ScrollContainerFixture />
) : searchParams.has("search") ? (
  <SearchPanelFixture />
) : (
  <FloatingPanelFixture />
);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  fixture,
);
