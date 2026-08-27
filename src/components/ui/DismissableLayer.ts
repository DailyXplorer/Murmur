import { useCallback, useLayoutEffect, useRef } from "react";

type EscapeHandler = (event: KeyboardEvent) => void;

interface LayerRegistration {
  id: symbol;
  sequence: number;
  zIndex: number;
  onEscape?: EscapeHandler;
}

const layers = new Map<symbol, LayerRegistration>();
let nextSequence = 0;
let isListening = false;

const topmostLayer = () =>
  Array.from(layers.values()).reduce<LayerRegistration | null>(
    (topmost, layer) => {
      if (
        !topmost ||
        layer.zIndex > topmost.zIndex ||
        (layer.zIndex === topmost.zIndex && layer.sequence > topmost.sequence)
      ) {
        return layer;
      }
      return topmost;
    },
    null,
  );

const handleDocumentKeyDown = (event: KeyboardEvent) => {
  if (event.key !== "Escape" || event.defaultPrevented) return;
  topmostLayer()?.onEscape?.(event);
};

const syncDocumentListener = () => {
  if (layers.size > 0 && !isListening) {
    document.addEventListener("keydown", handleDocumentKeyDown);
    isListening = true;
  } else if (layers.size === 0 && isListening) {
    document.removeEventListener("keydown", handleDocumentKeyDown);
    isListening = false;
  }
};

/** Registers a portaled layer in visual stacking order. */
export const useDismissableLayer = (
  active: boolean,
  zIndex: number,
  onEscape?: EscapeHandler,
) => {
  const idRef = useRef(Symbol("dismissable-layer"));
  const onEscapeRef = useRef(onEscape);

  useLayoutEffect(() => {
    onEscapeRef.current = onEscape;
  }, [onEscape]);

  useLayoutEffect(() => {
    if (!active) return;

    const id = idRef.current;
    layers.set(id, {
      id,
      sequence: nextSequence++,
      zIndex,
      onEscape: (event) => onEscapeRef.current?.(event),
    });
    syncDocumentListener();

    return () => {
      layers.delete(id);
      syncDocumentListener();
    };
  }, [active, zIndex]);

  return useCallback(() => topmostLayer()?.id === idRef.current, []);
};
