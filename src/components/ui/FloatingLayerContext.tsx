import { createContext, useContext } from "react";
import { LAYER_Z_INDEX } from "@/lib/constants/layers";

interface FloatingLayerValues {
  floatingContent: number;
  tooltip: number;
}

const DEFAULT_FLOATING_LAYERS: FloatingLayerValues = {
  floatingContent: LAYER_Z_INDEX.floatingContent,
  tooltip: LAYER_Z_INDEX.tooltip,
};

export const DIALOG_FLOATING_LAYERS: FloatingLayerValues = {
  floatingContent: LAYER_Z_INDEX.dialogFloatingContent,
  tooltip: LAYER_Z_INDEX.dialogTooltip,
};

export const FloatingLayerContext = createContext<FloatingLayerValues>(
  DEFAULT_FLOATING_LAYERS,
);

export const useFloatingLayers = () => useContext(FloatingLayerContext);
