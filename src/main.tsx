import React from "react";
import ReactDOM from "react-dom/client";
import { FluentProvider, webDarkTheme } from "@fluentui/react-components";
import App from "./App";
import "./index.css";

// Tema escuro do Fluent; o fundo fica no tom slate dos cartões do app.
const theme = { ...webDarkTheme, colorNeutralBackground1: "#0f172a" };

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <FluentProvider theme={theme} className="h-full">
      <App />
    </FluentProvider>
  </React.StrictMode>,
);
