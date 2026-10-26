interface RadioOptionProps<T extends string> {
  name: string;
  value: T;
  checked: boolean;
  disabled: boolean;
  title: string;
  description?: string;
  onSelect: (value: T) => void;
  /** The padding, which sets the option's click target. */
  className?: string;
}

/** One choice of a `role="radiogroup"`: a title, over a short description
 * when there is one. */
export const RadioOption = <T extends string>({
  name,
  value,
  checked,
  disabled,
  title,
  description,
  onSelect,
  className = "px-4 py-2.5",
}: RadioOptionProps<T>) => (
  <label
    className={`flex items-start gap-3 ${className} ${disabled ? "cursor-not-allowed" : "cursor-pointer"}`}
  >
    <input
      type="radio"
      name={name}
      value={value}
      checked={checked}
      disabled={disabled}
      onChange={() => onSelect(value)}
      className="mt-0.5 size-4 shrink-0 cursor-[inherit] appearance-none rounded-full border border-mid-gray/40 bg-mid-gray/20 transition-shadow duration-200 checked:border-transparent checked:bg-white checked:shadow-[inset_0_0_0_4px_var(--color-background-ui),var(--knob-shadow)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-text focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:opacity-50"
    />
    <span className={`min-w-0 ${disabled ? "opacity-50" : ""}`}>
      <span className="block text-sm font-medium">{title}</span>
      {description && (
        <span className="block text-pretty text-sm text-text/70">
          {description}
        </span>
      )}
    </span>
  </label>
);
