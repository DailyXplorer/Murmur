import React, { useCallback, useRef, useState } from "react";
import ReactDOM from "react-dom/client";
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
          onKeyDown={(event) => setLastInputKey(event.key)}
        />
        <button type="button" role="option">
          Search result
        </button>
      </FloatingPanel>
    </div>
  );
};

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

const FloatingPanelFixture: React.FC = () => {
  const [selectedValue, setSelectedValue] = useState<string | null>(null);

  return (
    <div className="h-screen bg-background text-text">
      <div data-testid="clipping-container" className="h-72 overflow-hidden">
        <div className="flex h-full items-end justify-center pb-2">
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
) : searchParams.has("search") ? (
  <SearchPanelFixture />
) : (
  <FloatingPanelFixture />
);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  fixture,
);
