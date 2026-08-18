import React from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./tokens.css";
import "./app.css";

const root = document.getElementById("root");
if (!root) throw new Error("the document has no #root to mount into");

// No StrictMode double-render outside development: it would issue every search twice, and the
// keystroke budget is measured against what actually happens.
createRoot(root).render(
  import.meta.env.DEV ? (
    <React.StrictMode>
      <App />
    </React.StrictMode>
  ) : (
    <App />
  ),
);
