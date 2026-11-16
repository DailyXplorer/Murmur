import React, {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { CaretDownIcon } from "@phosphor-icons/react/dist/csr/CaretDown";
import { useTranslation } from "react-i18next";
import { FloatingPanel } from "../ui/FloatingPanel";
import { Input } from "../ui/Input";
import {
  FIELD_SIZE_CLASSES,
  FIELD_SURFACE_CLASSES,
} from "../ui/controlClasses";

export interface LanguageMenuOption {
  value: string;
  /** Shown in the list. */
  name: string;
  /** Matched by the search too: the English name of a language. */
  label: string;
}

interface LanguageMenuProps {
  options: LanguageMenuOption[];
  selected: string;
  /** Shown on the trigger. */
  selectedName: string;
  onSelect: (value: string) => unknown;
  disabled?: boolean;
}

/** A field-sized trigger that opens a searchable, portaled list of
 * recognition languages. */
export const LanguageMenu: React.FC<LanguageMenuProps> = ({
  options,
  selected,
  selectedName,
  onSelect,
  disabled = false,
}) => {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const triggerRef = useRef<HTMLButtonElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const triggerId = useId();
  const menuId = useId();
  const isMenuOpen = isOpen && !disabled;

  useEffect(() => {
    if (isMenuOpen && searchInputRef.current) {
      searchInputRef.current.focus();
    }
  }, [isMenuOpen]);

  const filtered = useMemo(() => {
    const query = searchQuery.toLowerCase();
    return options.filter(
      (option) =>
        option.name.toLowerCase().includes(query) ||
        option.label.toLowerCase().includes(query),
    );
  }, [searchQuery, options]);
  const initialTabStopValue =
    filtered.find((option) => option.value === selected)?.value ??
    filtered[0]?.value;

  const handleSelect = async (value: string) => {
    await onSelect(value);
    setIsOpen(false);
    setSearchQuery("");
    // The save can take a while: the trigger gets the focus back only if
    // nothing else took it meanwhile.
    requestAnimationFrame(() => {
      const active = document.activeElement;
      if (!active || active === document.body) triggerRef.current?.focus();
    });
  };

  const handleToggle = () => {
    if (disabled) return;
    setIsOpen(!isOpen);
  };

  const handleDismiss = useCallback(() => {
    setIsOpen(false);
    setSearchQuery("");
  }, []);

  const handleKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter" && filtered.length > 0) {
      void handleSelect(filtered[0].value);
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      listRef.current
        ?.querySelector<HTMLElement>('[role="option"]:not([disabled])')
        ?.focus();
    } else if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      handleDismiss();
      triggerRef.current?.focus();
    }
  };

  return (
    <div className="relative min-w-0 flex-1">
      <button
        ref={triggerRef}
        id={triggerId}
        type="button"
        data-control="field"
        className={`${FIELD_SIZE_CLASSES} ${FIELD_SURFACE_CLASSES} flex w-full min-w-[200px] items-center justify-between transition-[background-color,border-color] duration-150 ${
          disabled
            ? "opacity-50 cursor-not-allowed"
            : "hover:bg-logo-primary/10 cursor-pointer hover:border-logo-primary"
        }`}
        onClick={handleToggle}
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={isMenuOpen}
        aria-controls={isMenuOpen ? menuId : undefined}
      >
        <span className="truncate">{selectedName}</span>
        <CaretDownIcon
          size={14}
          className={`ms-2 transition-transform duration-200 ${
            isMenuOpen ? "transform rotate-180" : ""
          }`}
        />
      </button>

      <FloatingPanel
        open={isMenuOpen}
        anchorRef={triggerRef}
        onDismiss={handleDismiss}
        className="flex flex-col overflow-hidden rounded-md border border-mid-gray/80 bg-background shadow-lg"
      >
        <div className="p-2 border-b border-mid-gray/80">
          <Input
            ref={searchInputRef}
            type="text"
            value={searchQuery}
            onChange={(event) => setSearchQuery(event.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={t("settings.general.language.searchPlaceholder")}
            className="w-full"
          />
        </div>

        <div
          ref={listRef}
          id={menuId}
          role="listbox"
          aria-labelledby={triggerId}
          className="min-h-0 flex-1 overflow-y-auto"
        >
          {filtered.length === 0 ? (
            <div className="px-2 py-2 text-sm text-mid-gray text-center">
              {t("settings.general.language.noResults")}
            </div>
          ) : (
            filtered.map((option) => (
              <button
                key={option.value}
                type="button"
                role="option"
                aria-selected={selected === option.value}
                tabIndex={option.value === initialTabStopValue ? 0 : -1}
                className={`min-h-control w-full px-3 py-1 text-sm font-normal text-start hover:bg-logo-primary/10 transition-colors duration-150 ${
                  selected === option.value
                    ? "bg-logo-primary/20 text-accent-text"
                    : ""
                }`}
                onClick={() => void handleSelect(option.value)}
              >
                <div className="flex items-center justify-between">
                  <span className="truncate">{option.name}</span>
                </div>
              </button>
            ))
          )}
        </div>
      </FloatingPanel>
    </div>
  );
};
