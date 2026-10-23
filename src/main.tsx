import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { QueryBuilderWindow } from "./components/querybuilder/QueryBuilderWindow";
import { DataTransferWindow } from "./components/transfer/DataTransferWindow";
import "./styles/tokens.css";
import "./styles/shell.css";
import "./styles/components.css";

// A janela do Query Builder é aberta pelo Rust com os parâmetros em `window.__ATLAS_QB__`.
const builder = (window as { __ATLAS_QB__?: unknown }).__ATLAS_QB__;
const transfer = (window as { __ATLAS_DT__?: unknown }).__ATLAS_DT__;

createRoot(document.getElementById("root")!).render(<StrictMode>{builder ? <QueryBuilderWindow /> : transfer ? <DataTransferWindow /> : <App />}</StrictMode>);
