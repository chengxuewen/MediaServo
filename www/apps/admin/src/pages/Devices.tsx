import { useCallback, useEffect, useState } from 'react';
import { getAdminDevices, registerDevice, revokeDevice, resetDeviceSecret } from '../api/client';
import type { AdminDevice } from '../api/client';
import Modal from '../components/Modal';
import './Devices.css';

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
              <span className="device-admin-name">🖥️ {d.device_id}</span>
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
          <p className="secret-warning">⚠️ This secret is shown only once — save it now.</p>
          <textarea
            className="secret-box"
            readOnly
            value={secretModal.secret}
            rows={3}
            onFocus={(e) => e.target.select()}
          />
          {copyError && <p className="form-error">{copyError}</p>}
          <div className="form-actions">
            <button className="btn" onClick={handleCopy}>{copied ? '✅ Copied' : 'Copy'}</button>
            <button className="btn btn-outline" onClick={() => setSecretModal(null)}>Done</button>
          </div>
        </Modal>
      )}
    </div>
  );
}