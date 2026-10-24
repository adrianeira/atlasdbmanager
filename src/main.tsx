import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { QueryBuilderWindow } from "./components/querybuilder/QueryBuilderWindow";
import { DataDictionaryWindow } from "./components/transfer/DataDictionaryWindow";
import { DataGenWindow } from "./components/transfer/DataGenWindow";
import { DataSyncWindow } from "./components/transfer/DataSyncWindow";
import { DataTransferWindow } from "./components/transfer/DataTransferWindow";
import { StructSyncWindow } from "./components/transfer/StructSyncWindow";
import { disableAutofill } from "./noAutofill";
import "./styles/tokens.css";
import "./styles/shell.css";
import "./styles/components.css";

// A janela do Query Builder é aberta pelo Rust com os parâmetros em `window.__ATLAS_QB__`.
const builder = (window as { __ATLAS_QB__?: unknown }).__ATLAS_QB__;
const transfer = (window as { __ATLAS_DT__?: unknown }).__ATLAS_DT__;
const sync = (window as { __ATLAS_DS__?: unknown }).__ATLAS_DS__;
const structure = (window as { __ATLAS_SS__?: unknown }).__ATLAS_SS__;
const generation = (window as { __ATLAS_DG__?: unknown }).__ATLAS_DG__;
const dictionary = (window as { __ATLAS_DD__?: unknown }).__ATLAS_DD__;

disableAutofill();
createRoot(document.getElementById("root")!).render(<StrictMode>{builder ? <QueryBuilderWindow /> : transfer ? <DataTransferWindow /> : sync ? <DataSyncWindow /> : structure ? <StructSyncWindow /> : generation ? <DataGenWindow /> : dictionary ? <DataDictionaryWindow /> : <App />}</StrictMode>);
