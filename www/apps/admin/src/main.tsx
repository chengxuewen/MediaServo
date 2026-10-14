import React from 'react';
import ReactDOM from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import App from './App';
import './App.css';

// When served at /admin, set basename for proper routing
const basename = window.location.pathname.startsWith('/admin') ? '/admin' : '/';

// Branded title (injected at compile time via vite define — changing env at runtime has no effect; needs pnpm build + server rebuild, C24)
declare const __APP_TITLE__: string;
document.title = __APP_TITLE__;

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <BrowserRouter basename={basename}>
      <App />
    </BrowserRouter>
  </React.StrictMode>,
);
