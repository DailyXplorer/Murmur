import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { useFloatingLayers } from "./FloatingLayerContext";

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
  const restoreFocusFrameRef = useRef<number | null>(null);
  const [position, setPosition] = useState<FloatingPanelPosition | null>(null);

  const updatePosition = useCallback(() => {
    const anchor = anchorRef.current;
    const panel = panelRef.current;
    if (!anchor || !panel) return;

    const anchorRect = anchor.getBoundingClientRect();
    const anchorIsOutsideViewport =
      anchorRect.bottom <= viewportPadding ||
      anchorRect.top >= window.innerHeight - viewportPadding ||
      anchorRect.right <= viewportPadding ||
      anchorRect.left >= window.innerWidth - viewportPadding;
    if (anchorIsOutsideViewport) {
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

    if (restoreFocusFrameRef.current !== null) {
      cancelAnimationFrame(restoreFocusFrameRef.current);
      restoreFocusFrameRef.current = null;
    }
    shouldRestoreFocusRef.current =
      panelRef.current?.contains(document.activeElement) ?? false;

    const focusableOptions = () =>
      Array.from(
        panelRef.current?.querySelectorAll<HTMLElement>(
          '[role="option"]:not([disabled])',
        ) ?? [],
      );
    const isTopmostPanel = () => {
      const floatingPanels = document.querySelectorAll<HTMLElement>(
        "[data-floating-panel-root]",
      );
      return (
        floatingPanels.item(floatingPanels.length - 1) === panelRef.current
      );
    };
    const handleMouseDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (
        isTopmostPanel() &&
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
      } else if (!anchorRef.current?.contains(target)) {
        shouldRestoreFocusRef.current = false;
        if (isTopmostPanel()) onDismiss();
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
        options[nextIndex]?.focus();
        return;
      }

      if (
        event.key === "Escape" &&
        !event.defaultPrevented &&
        isTopmostPanel()
      ) {
        event.preventDefault();
        shouldRestoreFocusRef.current = false;
        onDismiss();
        anchorRef.current?.focus();
      }
    };

    const focusFrame = focusFirstOptionOnOpen
      ? requestAnimationFrame(() => focusableOptions()[0]?.focus())
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
        restoreFocusFrameRef.current = requestAnimationFrame(() => {
          restoreFocusFrameRef.current = null;
          if (document.activeElement === document.body) {
            anchorRef.current?.focus();
          }
        });
      }
    };
  }, [anchorRef, focusFirstOptionOnOpen, onDismiss, open]);

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
