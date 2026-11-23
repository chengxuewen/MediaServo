import React from 'react';
import ReactDOM from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import App from './App';
import '@mediaservo/ui/tokens.css';
import '@mediaservo/react/react.css';
import './App.css';

// Served at /monitor (caddy same-origin, D10=B) → set basename for proper routing;
// dev runs at root. Mirrors the admin basename heuristic.
const basename = window.location.pathname.startsWith('/monitor') ? '/monitor' : '/';

// Branded title (injected at compile time via vite define — changing env at runtime
// has no effect; needs pnpm build + server rebuild, C24).
declare const __APP_TITLE__: string;
document.title = __APP_TITLE__;

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <BrowserRouter basename={basename}>
      <App />
    </BrowserRouter>
  </React.StrictMode>,
);