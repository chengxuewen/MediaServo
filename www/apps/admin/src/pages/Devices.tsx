import { useCallback, useEffect, useState } from 'react';
import { getAdminDevices, registerDevice, revokeDevice, resetDeviceSecret, getPendingDevices, approvePendingDevice } from '../api/client';
import type { AdminDevice, PendingDevice } from '../api/client';
import Modal from '../components/Modal';
import './Devices.css';
import { MonitorCog, AlertTriangle, Check, Inbox } from 'lucide-react';

// 首见时间 → 相对时间（刻度仿 Vehicles fmtUptime）
function fmtAgo(firstSeenMs: number): string {
  const secs = Math.max(0, Math.floor((Date.now() - firstSeenMs) / 1000));
  if (secs < 60) return `${secs}s 前`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m 前`;
  return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m 前`;
}

export default function Devices() {
  const [devices, setDevices] = useState<AdminDevice[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionMsg, setActionMsg] = useState<{ type: 'success' | 'error'; text: string } | null>(null);

  // 注册表单弹窗
  const [registerOpen, setRegisterOpen] = useState(false);
  const [newDeviceId, setNewDeviceId] = useState('');
  const [providedSecret, setProvidedSecret] = useState('');
  const [registerError, setRegisterError] = useState<string | null>(null);
  const [registering, setRegistering] = useState(false);

  // 一次性 secret 展示（注册/重置后弹出，关闭即弃 — 不落 state 之外）
  const [secretModal, setSecretModal] = useState<{ title: string; secret: string } | null>(null);
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState<string | null>(null);

  // 吊销/重置进行中的行（防重复点击）
  const [busy, setBusy] = useState<string | null>(null);

  // device-enroll: 待批准队列（手动档；内存表 D-E5，重启即清）
  const [pending, setPending] = useState<PendingDevice[]>([]);
  const [pendingError, setPendingError] = useState<string | null>(null);
  const [pendingNames, setPendingNames] = useState<Record<string, string>>({});
  const [pendingBusy, setPendingBusy] = useState<string | null>(null);

  const fetchDevices = useCallback(async () => {
    try {
      const data = await getAdminDevices();
      setDevices(data.devices);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch devices');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { fetchDevices(); }, [fetchDevices]);

  const fetchPending = useCallback(async () => {
    try {
      const data = await getPendingDevices();
      setPending(data.pending);
      setPendingError(null);
    } catch (e) {
      setPendingError(e instanceof Error ? e.message : 'Failed to fetch pending devices');
    }
  }, []);

  useEffect(() => { fetchPending(); }, [fetchPending]);
  // ponytail: 5s 轮询（与 useDevices/Vehicles 同节奏）
  useEffect(() => {
    const interval = setInterval(fetchPending, 5000);
    return () => clearInterval(interval);
  }, [fetchPending]);

  const openRegister = () => {
    setRegisterOpen(true);
    setNewDeviceId('');
    setProvidedSecret('');
    setRegisterError(null);
  };

  const handleRegister = async () => {
    const id = newDeviceId.trim();
    if (!id) { setRegisterError('Device ID is required'); return; }
    if (registering) return;
    setRegistering(true);
    setRegisterError(null);
    try {
      const resp = await registerDevice(id, providedSecret);
      setRegisterOpen(false);
      setSecretModal({ title: `Registered ${resp.device_id}`, secret: resp.secret });
      setCopied(false);
      setCopyError(null);
      fetchDevices();
    } catch (e) {
      setRegisterError(e instanceof Error ? e.message : 'Registration failed');
    } finally {
      setRegistering(false);
    }
  };

  const handleRevoke = async (deviceId: string) => {
    if (!window.confirm(`Revoke device ${deviceId}? This cannot be undone.`)) return;
    setBusy(deviceId);
    try {
      await revokeDevice(deviceId);
      setActionMsg({ type: 'success', text: `Device ${deviceId} revoked` });
      fetchDevices();
    } catch (e) {
      setActionMsg({ type: 'error', text: e instanceof Error ? e.message : 'Revoke failed' });
    } finally {
      setBusy(null);
    }
  };

  const handleResetSecret = async (deviceId: string) => {
    if (!window.confirm(`Reset the secret for ${deviceId}?`)) return;
    setBusy(deviceId);
    try {
      const resp = await resetDeviceSecret(deviceId);
      setSecretModal({ title: `New key for ${resp.device_id}`, secret: resp.secret });
      setCopied(false);
      setCopyError(null);
    } catch (e) {
      setActionMsg({ type: 'error', text: e instanceof Error ? e.message : 'Reset failed' });
    } finally {
      setBusy(null);
    }
  };

  const handleApprove = async (deviceId: string) => {
    if (pendingBusy) return;
    setPendingBusy(deviceId);
    try {
      const resp = await approvePendingDevice(deviceId, pendingNames[deviceId]);
      setPendingNames((prev) => { const next = { ...prev }; delete next[deviceId]; return next; });
      setActionMsg({ type: 'success', text: `${resp.device_id} 已批准 — ${resp.note}` });
      fetchPending();
    } catch (e) {
      setActionMsg({ type: 'error', text: e instanceof Error ? e.message : 'Approve failed' });
    } finally {
      setPendingBusy(null);
    }
  };

  const handleCopy = async () => {
    if (!secretModal) return;
    try {
      await navigator.clipboard.writeText(secretModal.secret);
      setCopied(true);
    } catch {
      // fallback: secret 以只读框展示可全选手动复制 — 不假设 clipboard 可用
      setCopyError('Clipboard unavailable — select the secret manually');
    }
  };

  if (loading) return <div className="loading">Loading...</div>;
  if (error && devices.length === 0) return <div className="error">{error}</div>;

  return (
    <div className="devices">
      <div className="pending-card">
        <h2 className="section-title"><Inbox size={14} /> 待批准设备</h2>
        {pendingError && <p className="token-status error">{pendingError}</p>}
        {pending.length === 0 ? (
          <p className="empty">暂无待批准设备</p>
        ) : (
          <div className="pending-list">
            {pending.map((p) => (
              <div key={p.device_id} className="pending-row">
                <span className="pending-id">{p.device_id}</span>
                <span className="pending-seen">首见 {fmtAgo(p.first_seen_ms)}</span>
                <input
                  className="pending-name-input"
                  placeholder="名称（可选）"
                  value={pendingNames[p.device_id] ?? ''}
                  onChange={(e) => setPendingNames((prev) => ({ ...prev, [p.device_id]: e.target.value }))}
                />
                <button
                  className="btn-secret"
                  disabled={pendingBusy === p.device_id}
                  onClick={() => handleApprove(p.device_id)}
                >批准</button>
              </div>
            ))}
          </div>
        )}
      </div>

      <div className="devices-head">
        <h2 className="section-title">Registered Devices</h2>
        <button className="btn" onClick={openRegister}>+ Register Device</button>
      </div>

      {actionMsg && <p className={`token-status ${actionMsg.type === 'success' ? 'saved' : 'error'}`}>{actionMsg.text}</p>}
      {error && <p className="token-status error">{error}</p>}

      {devices.length === 0 ? (
        <p className="empty">No devices registered</p>
      ) : (
        <div className="device-admin-list">
          {devices.map((d) => (
            <div key={d.device_id} className="device-admin-row">
              <span className="device-admin-name"><MonitorCog size={14} /> {d.device_id}</span>
              <span className="device-admin-actions">
                <button className="btn-secret" disabled={busy === d.device_id} onClick={() => handleResetSecret(d.device_id)}>Reset Secret</button>
                <button className="btn-sm" disabled={busy === d.device_id} onClick={() => handleRevoke(d.device_id)}>Revoke</button>
              </span>
            </div>
          ))}
        </div>
      )}

      {registerOpen && (
        <Modal title="Register Device" onClose={() => setRegisterOpen(false)}>
          <div className="form-row">
            <label className="form-label" htmlFor="device-id">Device ID</label>
            <input
              id="device-id"
              className="form-field"
              value={newDeviceId}
              onChange={(e) => setNewDeviceId(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') handleRegister(); }}
              placeholder="vehicle-01"
            />
          </div>
          <div className="form-row">
            <label className="form-label" htmlFor="device-secret">
              Secret <span className="form-hint">(optional — blank = server generates)</span>
            </label>
            <input
              id="device-secret"
              className="form-field"
              type="password"
              value={providedSecret}
              onChange={(e) => setProvidedSecret(e.target.value)}
              placeholder="leave blank for auto-generate"
            />
          </div>
          {registerError && <p className="form-error">{registerError}</p>}
          <div className="form-actions">
            <button className="btn btn-outline" onClick={() => setRegisterOpen(false)}>Cancel</button>
            <button className="btn" onClick={handleRegister} disabled={registering}>
              {registering ? 'Registering...' : 'Register'}
            </button>
          </div>
        </Modal>
      )}

      {secretModal && (
        <Modal title={secretModal.title} onClose={() => setSecretModal(null)}>
          <p className="secret-warning"><AlertTriangle size={14} /> This secret is shown only once — save it now.</p>
          <textarea
            className="secret-box"
            readOnly
            value={secretModal.secret}
            rows={3}
            onFocus={(e) => e.target.select()}
          />
          {copyError && <p className="form-error">{copyError}</p>}
          <div className="form-actions">
            <button className="btn" onClick={handleCopy}>{copied ? <><Check size={12} /> Copied</> : 'Copy'}</button>
            <button className="btn btn-outline" onClick={() => setSecretModal(null)}>Done</button>
          </div>
        </Modal>
      )}
    </div>
  );
}