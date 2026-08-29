import React, { useCallback, useEffect, useId, useRef, useState } from "react";
import { InfoIcon } from "@phosphor-icons/react/dist/csr/Info";
import { Tooltip } from "./Tooltip";

interface SettingContainerProps {
  title: string;
  description: string;
  children: React.ReactNode;
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  layout?: "horizontal" | "stacked";
  disabled?: boolean;
  tooltipPosition?: "top" | "bottom";
}

interface DescriptionTooltipProps {
  description: string;
  position: "top" | "bottom";
}

const TOOLTIP_HIDE_DELAY_MS = 150;

const DescriptionTooltip: React.FC<DescriptionTooltipProps> = ({
  description,
  position,
}) => {
  const [visible, setVisible] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const hideTimeoutRef = useRef<number | null>(null);
  const tooltipId = useId();

  const clearHideTimeout = useCallback(() => {
    if (hideTimeoutRef.current === null) return;

    window.clearTimeout(hideTimeoutRef.current);
    hideTimeoutRef.current = null;
  }, []);

  const showTooltip = useCallback(() => {
    clearHideTimeout();
    setVisible(true);
  }, [clearHideTimeout]);

  const hideTooltip = useCallback(() => {
    clearHideTimeout();
    setVisible(false);
  }, [clearHideTimeout]);

  const scheduleTooltipHide = useCallback(() => {
    clearHideTimeout();
    hideTimeoutRef.current = window.setTimeout(() => {
      hideTimeoutRef.current = null;
      setVisible(false);
    }, TOOLTIP_HIDE_DELAY_MS);
  }, [clearHideTimeout]);

  useEffect(() => clearHideTimeout, [clearHideTimeout]);

  useEffect(() => {
    if (!visible) return;

    const handleClickOutside = (event: MouseEvent) => {
      if (
        triggerRef.current &&
        event.target instanceof Node &&
        !triggerRef.current.contains(event.target)
      ) {
        hideTooltip();
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") hideTooltip();
    };

    document.addEventListener("mousedown", handleClickOutside);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [hideTooltip, visible]);

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        aria-label={description}
        aria-describedby={visible ? tooltipId : undefined}
        className="relative -my-1 flex size-10 shrink-0 items-center justify-center rounded-md text-mid-gray transition-colors duration-150 hover:text-logo-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
        onMouseEnter={showTooltip}
        onMouseLeave={scheduleTooltipHide}
        onFocus={showTooltip}
        onBlur={hideTooltip}
        onClick={showTooltip}
      >
        <InfoIcon size={15} aria-hidden="true" />
      </button>
      {visible && (
        <Tooltip
          id={tooltipId}
          targetRef={triggerRef}
          position={position}
          onMouseEnter={showTooltip}
          onMouseLeave={scheduleTooltipHide}
        >
          <p className="text-center text-sm leading-relaxed">{description}</p>
        </Tooltip>
      )}
    </>
  );
};

export const SettingContainer: React.FC<SettingContainerProps> = ({
  title,
  description,
  children,
  descriptionMode = "tooltip",
  grouped = false,
  layout = "horizontal",
  disabled = false,
  tooltipPosition = "top",
}) => {
  const titleClasses = `min-w-0 text-pretty text-sm font-medium ${disabled ? "opacity-50" : ""}`;
  const containerClasses = grouped
    ? "px-4 p-2"
    : "rounded-lg border border-mid-gray/20 px-4 p-2";

  if (layout === "stacked") {
    return (
      <div className={containerClasses}>
        <div className="mb-2 flex min-w-0 items-center gap-1">
          <h3 className={titleClasses}>{title}</h3>
          {descriptionMode === "tooltip" && (
            <DescriptionTooltip
              description={description}
              position={tooltipPosition}
            />
          )}
        </div>
        {descriptionMode === "inline" && (
          <p
            className={`mb-2 text-pretty text-sm text-text/70 ${disabled ? "opacity-50" : ""}`}
          >
            {description}
          </p>
        )}
        <div className="w-full">{children}</div>
      </div>
    );
  }

  const horizontalContainerClasses = grouped
    ? "flex min-h-12 items-center justify-between gap-4 px-4 py-1"
    : "flex min-h-12 items-center justify-between gap-4 rounded-lg border border-mid-gray/20 px-4 py-1";

  return (
    <div className={horizontalContainerClasses}>
      <div className="min-w-0 max-w-2/3">
        <div className="flex min-w-0 items-center gap-1">
          <h3 className={titleClasses}>{title}</h3>
          {descriptionMode === "tooltip" && (
            <DescriptionTooltip
              description={description}
              position={tooltipPosition}
            />
          )}
        </div>
        {descriptionMode === "inline" && (
          <p
            className={`text-pretty text-sm text-text/70 ${disabled ? "opacity-50" : ""}`}
          >
            {description}
          </p>
        )}
      </div>
      <div className="relative w-[var(--settings-control-rail-width)] shrink-0 text-end">
        {children}
      </div>
    </div>
  );
};
