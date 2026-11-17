import { createContext } from "react";

export const OpenLinkContext = createContext<((url: string) => void) | null>(null);
