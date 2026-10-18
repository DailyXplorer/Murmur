import React from "react";
import { ArrowCounterClockwiseIcon } from "@phosphor-icons/react/dist/csr/ArrowCounterClockwise";
import { IconButton } from "./IconButton";

interface ResetButtonProps {
  onClick: () => void;
  disabled?: boolean;
  className?: string;
  ariaLabel: string;
  children?: React.ReactNode;
}

export const ResetButton: React.FC<ResetButtonProps> = React.memo(
  ({ onClick, disabled = false, className = "", ariaLabel, children }) => (
    <IconButton
      label={ariaLabel}
      className={className}
      onClick={onClick}
      disabled={disabled}
    >
      {children ?? <ArrowCounterClockwiseIcon size={18} />}
    </IconButton>
  ),
);
