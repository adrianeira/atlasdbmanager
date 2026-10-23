import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { QueryBuilderWindow } from "./components/querybuilder/QueryBuilderWindow";
import "./styles/tokens.css";
import "./styles/shell.css";
import "./styles/components.css";

// A janela do Query Builder é aberta pelo Rust com os parâmetros em `window.__ATLAS_QB__`.
const builder = (window as { __ATLAS_QB__?: unknown }).__ATLAS_QB__;

createRoot(document.getElementById("root")!).render(<StrictMode>{builder ? <QueryBuilderWindow /> : <App />}</StrictMode>);
