import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./i18n";
import "@xyflow/react/dist/style.css";
import "./styles.css";
import { installOmarchyTheme } from "./omarchyTheme";

installOmarchyTheme();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
