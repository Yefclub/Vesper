import React from "react";
import ReactDOM from "react-dom/client";
import { Overlay } from "./Overlay";
import "../styles/index.css";

// The same stylesheet as the main window, so the card is in the app's family
// and follows the theme the boot script already applied to this document.
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Overlay />
  </React.StrictMode>,
);
