import React from "react";

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  /** `text` buttons all share one width, so buttons side by side match; a
   * label that does not fit (in a long locale) widens its button. `chip` fits
   * its content. Icon-only actions use `IconButton`. */
  size?: "text" | "chip";
}

const SIZE_CLASSES = {
  text: "h-control min-w-control-button px-4",
  chip: "h-control px-3",
} as const;

/** The one filled, accent-coloured button, destructive actions included. */
export const Button: React.FC<ButtonProps> = ({
  children,
  className = "",
  size = "text",
  ...props
}) => (
  <button
    data-control={size}
    className={`inline-flex items-center justify-center gap-2 whitespace-nowrap text-sm font-medium rounded-md border focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-text focus-visible:ring-offset-2 focus-visible:ring-offset-background transition-[color,background-color,border-color,transform] active:scale-[0.96] disabled:opacity-50 disabled:cursor-not-allowed disabled:active:scale-100 cursor-pointer text-on-accent bg-background-ui border-background-ui hover:bg-background-ui-hover hover:border-background-ui-hover active:bg-background-ui-active active:border-background-ui-active focus:ring-2 focus:ring-text focus:ring-offset-2 focus:ring-offset-background ${SIZE_CLASSES[size]} ${className}`}
    {...props}
  >
    {children}
  </button>
);
