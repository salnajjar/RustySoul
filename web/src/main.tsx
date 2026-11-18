import React from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { InstallAppProvider } from "./InstallApp";
import "./style.css";
import "./themes.css";

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <InstallAppProvider>
      <App />
    </InstallAppProvider>
  </React.StrictMode>,
);
