import { useEffect, useState } from 'react';
import { useLocation } from 'react-router-dom';
import Devices from './Devices';
import Accounts from './Accounts';
import ApiKeys from './ApiKeys';
import Vehicles from './Vehicles';
import Settings from './Settings';
import { useAuth } from '../hooks/useAuth';

/**
 * Admin hub (D12=A): the prototype shows ONE Administration entry; the spokes
 * live inside. Spoke visuals restyled per B1–B4 scoped approvals (2026-11-23,
 * PLAN §11); deferred layers (drawer/provider/etc.) stay out.
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
  const location = useLocation();
  const visible = TABS.filter((t) => !t.admin || isAdmin);
  // deep link: overview cards navigate here with { tab } (fleet lens / device
  // wall); B4 scope cross-link adds { query } to prefill the Devices search.
  const state = location.state as { tab?: string; query?: string } | null;
  const requested = state?.tab;
  const initial = visible.some((t) => t.id === requested) ? requested! : visible[0]?.id ?? 'settings';
  const [tab, setTab] = useState(initial);
  // same-route deep links (scope cross-link) update state without a remount
  useEffect(() => {
    const t = (location.state as { tab?: string } | null)?.tab;
    if (t && visible.some((v) => v.id === t)) setTab(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [location.state]);
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
      <div role="tabpanel">
        {active?.id === 'devices' ? <Devices initialQuery={state?.query ?? ''} /> : active?.el}
      </div>
    </section>
  );
}
