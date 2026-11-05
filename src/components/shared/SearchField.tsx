import React from "react";
import { useTranslation } from "react-i18next";
import { MagnifyingGlassIcon } from "@phosphor-icons/react/dist/csr/MagnifyingGlass";
import { XIcon } from "@phosphor-icons/react/dist/csr/X";
import { IconButton } from "../ui/IconButton";
import { Input } from "../ui/Input";

interface SearchFieldProps {
  value: string;
  onChange: (value: string) => void;
  /** Also the field's accessible name. */
  placeholder: string;
}

/** A search box: Escape or the clear button empties it. */
export const SearchField: React.FC<SearchFieldProps> = ({
  value,
  onChange,
  placeholder,
}) => {
  const { t } = useTranslation();
  return (
    <div className="relative">
      <MagnifyingGlassIcon
        size={15}
        aria-hidden="true"
        className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-mid-gray"
      />
      <Input
        type="text"
        className="w-full pl-9 pr-10"
        value={value}
        placeholder={placeholder}
        aria-label={placeholder}
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape" && value !== "") {
            event.preventDefault();
            onChange("");
          }
        }}
      />
      {value !== "" && (
        <IconButton
          size="compact"
          label={t("search.clear")}
          className="absolute right-1 top-1/2 -translate-y-1/2"
          onClick={() => onChange("")}
        >
          <XIcon size={15} aria-hidden="true" />
        </IconButton>
      )}
    </div>
  );
};
