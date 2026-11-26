import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "@fontsource/newsreader/400.css";
import "@fontsource/newsreader/600.css";
import "@fontsource/sora/400.css";
import "@fontsource/sora/500.css";
import "./index.css";
import { App } from "./App";
import { I18nProvider } from "./i18n";
import { UiProvider } from "./ui";

const stored = localStorage.getItem("biblioteka-theme");
if (stored === "dark" || (stored === null && matchMedia("(prefers-color-scheme: dark)").matches)) {
  document.documentElement.dataset.theme = "dark";
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <I18nProvider>
      <UiProvider>
        <App />
      </UiProvider>
    </I18nProvider>
  </StrictMode>,
);
