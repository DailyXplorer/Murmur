import React from "react";

interface IconButtonProps extends Omit<
  React.ButtonHTMLAttributes<HTMLButtonElement>,
  "aria-label"
> {
  /** Read by screen readers and shown as the tooltip. */
  label: string;
  /** `compact` sits in toolbars; `control` lines up with fields and buttons. */
  size?: "control" | "compact";
  /** A toggle that is on. */
  active?: boolean;
}

/** An unframed icon that takes a light accent tint on hover, never a border. */
export const IconButton: React.FC<IconButtonProps> = ({
  label,
  size = "control",
  active = false,
  className = "",
  children,
  ...props
}) => (
  <button
    type="button"
    aria-label={label}
    title={label}
    data-control={size === "control" ? "icon" : undefined}
    className={`inline-flex shrink-0 cursor-pointer items-center justify-center rounded-md transition-[color,background-color,transform] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary active:scale-[0.96] disabled:cursor-not-allowed disabled:bg-transparent disabled:text-text/25 disabled:active:scale-100 ${
      size === "control" ? "size-control" : "size-7"
    } ${
      active
        ? "bg-logo-primary/15 text-logo-primary hover:bg-logo-primary/25"
        : "text-text/60 hover:bg-logo-primary/15 hover:text-text"
    } ${className}`}
    {...props}
  >
    {children}
  </button>
);
