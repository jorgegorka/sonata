import { createContext, useContext } from "react";
import { api, type Project } from "./api";
export const ProjectApiContext = createContext(api);
export const ProjectContext = createContext<Project | null>(null);
export const useProjectApi = () => useContext(ProjectApiContext);
export const useProject = () => useContext(ProjectContext);
