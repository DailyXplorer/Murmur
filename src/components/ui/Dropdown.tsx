import React, { useCallback, useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { CaretDownIcon } from "@phosphor-icons/react/dist/csr/CaretDown";
import { FloatingPanel } from "./FloatingPanel";

export interface DropdownOption {
  value: string;
  label: string;
  disabled?: boolean;
}

interface DropdownProps {
  options: DropdownOption[];
  className?: string;
  selectedValue: string | null;
  onSelect: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  onRefresh?: () => void;
}

export const Dropdown: React.FC<DropdownProps> = ({
  options,
  selectedValue,
  onSelect,
  className = "",
  placeholder = "Select an option...",
  disabled = false,
  onRefresh,
}) => {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);
  const dropdownRef = useRef<HTMLButtonElement>(null);
  const triggerId = useId();
  const menuId = useId();
  const isDropdownOpen = isOpen && !disabled;

  useEffect(() => {
    if (disabled) setIsOpen(false);
  }, [disabled]);

  const selectedOption = options.find(
    (option) => option.value === selectedValue,
  );
  const initialTabStopValue =
    options.find((option) => option.value === selectedValue && !option.disabled)
      ?.value ?? options.find((option) => !option.disabled)?.value;

  const handleSelect = (value: string) => {
    onSelect(value);
    setIsOpen(false);
  };

  const handleToggle = () => {
    if (disabled) return;
    if (!isOpen && onRefresh) onRefresh();
    setIsOpen(!isOpen);
  };

  const handleDismiss = useCallback(() => setIsOpen(false), []);

  return (
    <div className={`relative ${className}`}>
      <button
        ref={dropdownRef}
        id={triggerId}
        type="button"
        className={`px-2 py-[5px] text-sm font-normal bg-mid-gray/10 border border-mid-gray/80 rounded-md min-w-[200px] w-full text-start grid grid-cols-[1fr_auto] gap-2 items-center transition-[background-color,border-color] duration-150 ${
          disabled
            ? "opacity-50 cursor-not-allowed"
            : "hover:bg-logo-primary/10 cursor-pointer hover:border-logo-primary"
        }`}
        onClick={handleToggle}
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={isDropdownOpen}
        aria-controls={isDropdownOpen ? menuId : undefined}
      >
        <span className="truncate">{selectedOption?.label || placeholder}</span>
        <CaretDownIcon
          size={14}
          className={`transition-transform duration-200 ${isDropdownOpen ? "transform rotate-180" : ""}`}
        />
      </button>
      <FloatingPanel
        open={isDropdownOpen}
        anchorRef={dropdownRef}
        onDismiss={handleDismiss}
        focusFirstOptionOnOpen={true}
        className="overflow-y-auto rounded-md border border-mid-gray/80 bg-background shadow-lg"
      >
        <div id={menuId} role="listbox" aria-labelledby={triggerId}>
          {options.length === 0 ? (
            <div className="px-2 py-1 text-sm text-mid-gray">
              {t("common.noOptionsFound")}
            </div>
          ) : (
            options.map((option) => (
              <button
                key={option.value}
                type="button"
                role="option"
                aria-selected={selectedValue === option.value}
                tabIndex={option.value === initialTabStopValue ? 0 : -1}
                className={`w-full px-2 py-1 text-sm font-normal text-start hover:bg-logo-primary/10 transition-colors duration-150 ${
                  selectedValue === option.value ? "bg-logo-primary/20" : ""
                } ${option.disabled ? "opacity-50 cursor-not-allowed" : ""}`}
                onClick={() => handleSelect(option.value)}
                disabled={option.disabled}
              >
                <span className="whitespace-normal break-words">
                  {option.label}
                </span>
              </button>
            ))
          )}
        </div>
      </FloatingPanel>
    </div>
  );
};
