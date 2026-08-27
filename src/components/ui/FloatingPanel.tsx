import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { useFloatingLayers } from "./FloatingLayerContext";
import { useDismissableLayer } from "./DismissableLayer";

type Placement = "top" | "bottom";

interface FloatingPanelPosition {
  top: number;
  left: number;
  width: number;
  maxHeight: number;
  placement: Placement;
}

interface FloatingPanelProps {
  open: boolean;
  anchorRef: React.RefObject<HTMLElement>;
  onDismiss: () => void;
  children: React.ReactNode;
  className?: string;
  maxHeight?: number;
  gap?: number;
  viewportPadding?: number;
  focusFirstOptionOnOpen?: boolean;
}

const DEFAULT_MAX_HEIGHT = 240;
const DEFAULT_GAP = 4;
const DEFAULT_VIEWPORT_PADDING = 8;
const FOCUSABLE_SELECTOR = [
  "a[href]",
  "button:not([disabled])",
  "textarea:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "[contenteditable='true']",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

const CLIPPING_OVERFLOW_VALUES = new Set(["auto", "clip", "hidden", "scroll"]);

const isVisible = (element: HTMLElement) => {
  const style = window.getComputedStyle(element);
  const rect = element.getBoundingClientRect();
  return (
    style.visibility !== "hidden" &&
    style.display !== "none" &&
    rect.width > 0 &&
    rect.height > 0
  );
};

const getFormTabStops = () =>
  Array.from(document.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
    (element) =>
      element.tabIndex >= 0 &&
      element.getAttribute("aria-hidden") !== "true" &&
      !element.closest("[data-floating-panel-root]") &&
      isVisible(element),
  );

const anchorIsOutsideClippingBounds = (
  anchor: HTMLElement,
  viewportPadding: number,
) => {
  const anchorRect = anchor.getBoundingClientRect();
  let top = viewportPadding;
  let right = window.innerWidth - viewportPadding;
  let bottom = window.innerHeight - viewportPadding;
  let left = viewportPadding;

  for (
    let ancestor = anchor.parentElement;
    ancestor;
    ancestor = ancestor.parentElement
  ) {
    if (ancestor === document.body || ancestor === document.documentElement) {
      continue;
    }

    const style = window.getComputedStyle(ancestor);
    const rect = ancestor.getBoundingClientRect();
    if (CLIPPING_OVERFLOW_VALUES.has(style.overflowX)) {
      left = Math.max(left, rect.left);
      right = Math.min(right, rect.right);
    }
    if (CLIPPING_OVERFLOW_VALUES.has(style.overflowY)) {
      top = Math.max(top, rect.top);
      bottom = Math.min(bottom, rect.bottom);
    }
  }

  return (
    anchorRect.bottom <= top ||
    anchorRect.top >= bottom ||
    anchorRect.right <= left ||
    anchorRect.left >= right
  );
};

const positionsMatch = (
  current: FloatingPanelPosition | null,
  next: FloatingPanelPosition,
) =>
  current?.top === next.top &&
  current.left === next.left &&
  current.width === next.width &&
  current.maxHeight === next.maxHeight &&
  current.placement === next.placement;

export const FloatingPanel: React.FC<FloatingPanelProps> = ({
  open,
  anchorRef,
  onDismiss,
  children,
  className = "",
  maxHeight = DEFAULT_MAX_HEIGHT,
  gap = DEFAULT_GAP,
  viewportPadding = DEFAULT_VIEWPORT_PADDING,
  focusFirstOptionOnOpen = false,
}) => {
  const { floatingContent: floatingContentZIndex } = useFloatingLayers();
  const panelRef = useRef<HTMLDivElement>(null);
  const shouldRestoreFocusRef = useRef(false);
  const cancelPendingRestoreRef = useRef<() => void>(() => undefined);
  const [position, setPosition] = useState<FloatingPanelPosition | null>(null);

  const handleEscape = useCallback(
    (event: KeyboardEvent) => {
      event.preventDefault();
      shouldRestoreFocusRef.current = false;
      onDismiss();
      anchorRef.current?.focus();
    },
    [anchorRef, onDismiss],
  );
  const isTopmostLayer = useDismissableLayer(
    open,
    floatingContentZIndex,
    handleEscape,
  );

  const updatePosition = useCallback(() => {
    const anchor = anchorRef.current;
    const panel = panelRef.current;
    if (!anchor || !panel) return;

    const anchorRect = anchor.getBoundingClientRect();
    if (anchorIsOutsideClippingBounds(anchor, viewportPadding)) {
      shouldRestoreFocusRef.current = false;
      onDismiss();
      return;
    }

    const availableWidth = Math.max(0, window.innerWidth - viewportPadding * 2);
    const width = Math.min(anchorRect.width, availableWidth);
    const left = Math.min(
      Math.max(anchorRect.left, viewportPadding),
      Math.max(viewportPadding, window.innerWidth - viewportPadding - width),
    );
    const desiredHeight = Math.min(panel.scrollHeight, maxHeight);
    const spaceBelow = Math.max(
      0,
      window.innerHeight - anchorRect.bottom - gap - viewportPadding,
    );
    const spaceAbove = Math.max(0, anchorRect.top - gap - viewportPadding);
    const placement: Placement =
      spaceBelow >= desiredHeight || spaceBelow >= spaceAbove
        ? "bottom"
        : "top";
    const placementSpace = placement === "bottom" ? spaceBelow : spaceAbove;
    const constrainedHeight = Math.min(desiredHeight, placementSpace);
    const top =
      placement === "bottom"
        ? anchorRect.bottom + gap
        : anchorRect.top - gap - constrainedHeight;
    const nextPosition = {
      top,
      left,
      width,
      maxHeight: placementSpace,
      placement,
    };

    setPosition((current) =>
      positionsMatch(current, nextPosition) ? current : nextPosition,
    );
  }, [anchorRef, gap, maxHeight, onDismiss, viewportPadding]);

  useLayoutEffect(() => {
    if (!open) {
      setPosition(null);
      return;
    }

    updatePosition();
    const animationFrame = requestAnimationFrame(updatePosition);
    const resizeObserver = new ResizeObserver(updatePosition);
    if (anchorRef.current) resizeObserver.observe(anchorRef.current);
    if (panelRef.current) resizeObserver.observe(panelRef.current);
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);

    return () => {
      cancelAnimationFrame(animationFrame);
      resizeObserver.disconnect();
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
    };
  }, [anchorRef, open, updatePosition]);

  useEffect(() => {
    if (!open) return;

    cancelPendingRestoreRef.current();
    cancelPendingRestoreRef.current = () => undefined;
    shouldRestoreFocusRef.current =
      panelRef.current?.contains(document.activeElement) ?? false;

    const focusableOptions = () =>
      Array.from(
        panelRef.current?.querySelectorAll<HTMLElement>(
          '[role="option"]:not([disabled])',
        ) ?? [],
      );
    const setRovingOption = (activeOption: HTMLElement) => {
      for (const option of focusableOptions()) {
        option.tabIndex = option === activeOption ? 0 : -1;
      }
    };
    const handleMouseDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (
        isTopmostLayer() &&
        !anchorRef.current?.contains(target) &&
        !panelRef.current?.contains(target)
      ) {
        shouldRestoreFocusRef.current = false;
        onDismiss();
      }
    };
    const handleFocusIn = (event: FocusEvent) => {
      const target = event.target as Node;
      if (panelRef.current?.contains(target)) {
        shouldRestoreFocusRef.current = true;
        if (
          target instanceof HTMLElement &&
          target.matches('[role="option"]:not([disabled])')
        ) {
          setRovingOption(target);
        }
      } else if (!anchorRef.current?.contains(target)) {
        shouldRestoreFocusRef.current = false;
        if (isTopmostLayer()) onDismiss();
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      const eventTarget = event.target;
      const isEnabledOption =
        eventTarget instanceof HTMLElement &&
        panelRef.current?.contains(eventTarget) &&
        eventTarget.matches('[role="option"]:not([disabled])');
      if (
        isEnabledOption &&
        ["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)
      ) {
        const options = focusableOptions();
        if (options.length === 0) return;

        const currentIndex = options.indexOf(
          document.activeElement as HTMLElement,
        );
        let nextIndex: number;
        if (event.key === "Home") {
          nextIndex = 0;
        } else if (event.key === "End") {
          nextIndex = options.length - 1;
        } else if (event.key === "ArrowUp") {
          nextIndex = currentIndex <= 0 ? options.length - 1 : currentIndex - 1;
        } else {
          nextIndex =
            currentIndex < 0 || currentIndex === options.length - 1
              ? 0
              : currentIndex + 1;
        }

        event.preventDefault();
        event.stopPropagation();
        const nextOption = options[nextIndex];
        if (nextOption) {
          setRovingOption(nextOption);
          nextOption.focus();
        }
        return;
      }

      if (
        event.key === "Tab" &&
        panelRef.current?.contains(eventTarget as Node) &&
        isTopmostLayer()
      ) {
        const tabStops = getFormTabStops();
        const anchor = anchorRef.current;
        const anchorIndex = anchor ? tabStops.indexOf(anchor) : -1;
        if (anchorIndex < 0 || tabStops.length < 2) return;

        const offset = event.shiftKey ? -1 : 1;
        const nextIndex =
          (anchorIndex + offset + tabStops.length) % tabStops.length;
        event.preventDefault();
        event.stopPropagation();
        shouldRestoreFocusRef.current = false;
        onDismiss();
        tabStops[nextIndex]?.focus();
      }
    };

    const focusFrame = focusFirstOptionOnOpen
      ? requestAnimationFrame(() => {
          const options = focusableOptions();
          const initialOption =
            options.find((option) => option.tabIndex === 0) ?? options[0];
          if (initialOption) {
            setRovingOption(initialOption);
            initialOption.focus();
          }
        })
      : null;

    document.addEventListener("mousedown", handleMouseDown);
    document.addEventListener("focusin", handleFocusIn);
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      if (focusFrame !== null) cancelAnimationFrame(focusFrame);
      document.removeEventListener("mousedown", handleMouseDown);
      document.removeEventListener("focusin", handleFocusIn);
      document.removeEventListener("keydown", handleKeyDown);
      const shouldRestoreFocus = shouldRestoreFocusRef.current;
      shouldRestoreFocusRef.current = false;
      if (shouldRestoreFocus) {
        const anchor = anchorRef.current;
        if (!anchor) return;

        let observer: MutationObserver | null = null;
        let timeout: number | null = null;
        const isAnchorDisabled = () =>
          anchor.matches(":disabled") ||
          anchor.getAttribute("aria-disabled") === "true";
        const stopWaiting = () => {
          observer?.disconnect();
          if (timeout !== null) window.clearTimeout(timeout);
          observer = null;
          timeout = null;
        };
        const tryRestore = () => {
          if (!anchor.isConnected || document.activeElement !== document.body) {
            stopWaiting();
            return;
          }
          if (!isAnchorDisabled()) {
            anchor.focus();
            stopWaiting();
          }
        };
        const restoreFrame = requestAnimationFrame(() => {
          tryRestore();
          if (
            anchor.isConnected &&
            document.activeElement === document.body &&
            isAnchorDisabled()
          ) {
            observer = new MutationObserver(tryRestore);
            observer.observe(anchor, {
              attributes: true,
              attributeFilter: ["disabled", "aria-disabled"],
            });
            timeout = window.setTimeout(stopWaiting, 5000);
            tryRestore();
          }
        });
        cancelPendingRestoreRef.current = () => {
          cancelAnimationFrame(restoreFrame);
          stopWaiting();
        };
      }
    };
  }, [anchorRef, focusFirstOptionOnOpen, isTopmostLayer, onDismiss, open]);

  useEffect(
    () => () => {
      cancelPendingRestoreRef.current();
    },
    [],
  );

  if (!open) return null;

  return createPortal(
    <div
      ref={panelRef}
      data-floating-panel-root
      data-placement={position?.placement}
      style={{
        position: "fixed",
        top: position?.top ?? -9999,
        left: position?.left ?? -9999,
        width: position?.width,
        maxHeight: Math.min(position?.maxHeight ?? maxHeight, maxHeight),
        zIndex: floatingContentZIndex,
        visibility: position ? "visible" : "hidden",
      }}
      className={className}
    >
      {children}
    </div>,
    document.body,
  );
};
