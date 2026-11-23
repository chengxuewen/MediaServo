import { useState } from 'react';
import Devices from './Devices';
import Accounts from './Accounts';
import ApiKeys from './ApiKeys';
import Vehicles from './Vehicles';
import Settings from './Settings';
import { useAuth } from '../hooks/useAuth';

/**
 * Admin hub (D12=A): the prototype shows ONE Administration entry; the spokes
 * live inside. This is the navigation skeleton -- spoke visuals still await
 * boards B1/B2/B3/B4 (H4), so the existing pages render unchanged for now.
 */
const TABS = [
  { id: 'devices', label: 'Devices', el: <Devices />, admin: true },
  { id: 'accounts', label: 'Accounts', el: <Accounts />, admin: true },
  { id: 'keys', label: 'API Keys', el: <ApiKeys />, admin: true },
  { id: 'fleet', label: 'Fleet', el: <Vehicles />, admin: false },
  { id: 'settings', label: 'Settings', el: <Settings />, admin: false },
];

export default function AdminHub() {
  const { isAdmin } = useAuth();
  const visible = TABS.filter((t) => !t.admin || isAdmin);
  const [tab, setTab] = useState(visible[0]?.id ?? 'settings');
  const active = visible.find((t) => t.id === tab) ?? visible[0];
  return (
    <section>
      <div className="hub-tabs" role="tablist" aria-label="Administration">
        {visible.map((t) => (
          <button
            key={t.id}
            role="tab"
            aria-selected={t.id === active?.id}
            className={t.id === active?.id ? 'hub-tab active' : 'hub-tab'}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div role="tabpanel">{active?.el}</div>
    </section>
  );
}
