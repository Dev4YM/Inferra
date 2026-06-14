import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { ReadmeHeroShowcase } from "@/dev/ReadmeHeroShowcase";
import "../index.css";

const el = document.getElementById("root");
if (el) {
  createRoot(el).render(
    <StrictMode>
      <ReadmeHeroShowcase />
    </StrictMode>,
  );
}
