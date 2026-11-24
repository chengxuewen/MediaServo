import React from 'react';
import ReactDOM from 'react-dom/client';
import { HashRouter } from 'react-router-dom';
import App from './App';
import '@mediaservo/ui/tokens.css';
import '@mediaservo/react/react.css';
import './App.css';

// HashRouter (with vite base './') routes via the URL fragment, so the dist loads
// under any root without a basename heuristic: caddy /monitor same-origin (D10=B)
// AND desktop file:// (Tauri/Electron direct load, desktop reservation). Deep links
// keep the path stable (/#/route) so relative assets always resolve — absolute
// base '/' would 404 assets under the /monitor subpath.

// Branded title (injected at compile time via vite define — changing env at runtime
// has no effect; needs pnpm build + server rebuild, C24).
declare const __APP_TITLE__: string;
document.title = __APP_TITLE__;

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <HashRouter>
      <App />
    </HashRouter>
  </React.StrictMode>,
);