import React from "react";
import ReactDOM from "react-dom/client";
import { Indicator } from "./Indicator";
import "../styles/index.css";

// The main window's stylesheet, so the indicator follows the same tokens and
// the theme the boot script already applied to this document.
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Indicator />
  </React.StrictMode>,
);
