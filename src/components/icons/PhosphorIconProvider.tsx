import { IconContext } from "@phosphor-icons/react/dist/lib/context";
import type { PropsWithChildren } from "react";

const PhosphorIconProvider = ({ children }: PropsWithChildren) => (
  <IconContext.Provider value={{ weight: "light" }}>
    {children}
  </IconContext.Provider>
);

export default PhosphorIconProvider;
