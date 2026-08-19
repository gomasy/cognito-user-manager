import { createRoot } from "react-dom/client";
import { App } from "./App";
import { init } from "./i18n";
import "./styles/index.scss";

const container = document.getElementById("root");

init()
  .catch(() => {
    // A missing catalog must not stop the app: t() falls back to the key.
  })
  .then(() => {
    if (container) createRoot(container).render(<App />);
  });
