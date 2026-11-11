import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { FieldPage } from './FieldPage';
import { DispatchPage } from './DispatchPage';

// No router dependency: the two pages are path-dispatched. The server (and
// vite dev) serve the same index.html for /voice/* paths; room comes from the
// path segment. Dev URLs:
//   http://localhost:5175/field/talk-car1
//   http://localhost:5175/dispatch/talk-car1
const path = window.location.pathname;
const parts = path.split('/').filter(Boolean);
const pageName = parts[0] === 'field' || parts[0] === 'dispatch' ? parts[0] : 'field';
const room = parts[1] ?? 'talk-car1';
const wsUrl = window.location.host;

const el = document.getElementById('root');
if (el) {
  createRoot(el).render(
    <StrictMode>
      {pageName === 'dispatch'
        ? <DispatchPage room={room} wsUrl={wsUrl} />
        : <FieldPage room={room} wsUrl={wsUrl} />}
    </StrictMode>,
  );
}
