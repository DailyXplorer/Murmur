import React from "react";

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  /** `text` buttons all share one width, so buttons side by side match; a
   * label that does not fit (in a long locale) widens its button. `chip` fits
   * its content. Icon-only actions use `IconButton`. */
  size?: "text" | "chip";
  /** `destructive` is the red confirm of an action that deletes something and
   * cannot be undone. Rows that open such a dialog stay `default`. */
  variant?: "default" | "destructive";
}

const SIZE_CLASSES = {
  text: "h-control min-w-control-button px-4",
  chip: "h-control px-3",
} as const;

const VARIANT_CLASSES = {
  default:
    "text-on-accent bg-background-ui border-background-ui hover:bg-background-ui-hover hover:border-background-ui-hover active:bg-background-ui-active active:border-background-ui-active",
  destructive:
    "text-on-error bg-error border-error hover:bg-error/90 hover:border-error/90 active:bg-error/80 active:border-error/80",
} as const;

/** The filled, accent-coloured button; `variant="destructive"` makes it red. */
export const Button: React.FC<ButtonProps> = ({
  children,
  className = "",
  size = "text",
  variant = "default",
  ...props
}) => (
  <button
    data-control={size}
    className={`inline-flex items-center justify-center gap-2 whitespace-nowrap text-sm font-medium rounded-md border focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-text focus-visible:ring-offset-2 focus-visible:ring-offset-background transition-[color,background-color,border-color,transform] active:scale-[0.96] disabled:opacity-50 disabled:cursor-not-allowed disabled:active:scale-100 cursor-pointer ${VARIANT_CLASSES[variant]} focus:ring-2 focus:ring-text focus:ring-offset-2 focus:ring-offset-background ${SIZE_CLASSES[size]} ${className}`}
    {...props}
  >
    {children}
  </button>
);
