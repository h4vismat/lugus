import { createRoot } from "react-dom/client";
import { TooltipProvider } from "./components/ui/tooltip";
import { App } from "./app";
import "./style.css";
createRoot(document.getElementById("root")!).render(
  <TooltipProvider>
    <App />
  </TooltipProvider>,
);
