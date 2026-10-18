import React, { createContext, useContext } from "react";

type IconButtonSize = "control" | "compact";

const ToolbarSize = createContext<IconButtonSize>("control");

interface IconButtonProps extends Omit<
  React.ButtonHTMLAttributes<HTMLButtonElement>,
  "aria-label"
> {
  /** Read by screen readers and shown as the tooltip. */
  label: string;
  /** `compact` sits in toolbars; `control` lines up with fields and buttons.
   * Inside an `IconToolbar` it is `compact` unless set. */
  size?: IconButtonSize;
  /** A toggle that is on. */
  active?: boolean;
}

/** An unframed icon that takes a light accent tint on hover, never a border. */
export const IconButton: React.FC<IconButtonProps> = ({
  label,
  size,
  active = false,
  className = "",
  children,
  ...props
}) => {
  const toolbarSize = useContext(ToolbarSize);
  const resolvedSize = size ?? toolbarSize;
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      data-control={resolvedSize === "control" ? "icon" : undefined}
      className={`inline-flex shrink-0 cursor-pointer items-center justify-center rounded-md transition-[color,background-color,transform] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary active:scale-[0.96] disabled:cursor-not-allowed disabled:bg-transparent disabled:text-text/25 disabled:active:scale-100 ${
        resolvedSize === "control" ? "size-control" : "size-7"
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
};

/**
 * A tight row of icon actions, such as copy, star and delete. Every group of
 * icon actions uses it, so they share one size, gap and hover.
 */
export const IconToolbar: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => (
  <ToolbarSize.Provider value="compact">
    <div className="flex shrink-0 items-center gap-1">{children}</div>
  </ToolbarSize.Provider>
);
