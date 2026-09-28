import { useCallback, useEffect, useState } from 'react';
import { getApiKeys, createApiKey, revokeApiKey } from '../api/client';
import type { AdminApiKey } from '../api/client';
import Modal from '../components/Modal';
import './ApiKeys.css';
import { AlertTriangle, Check, KeyRound } from 'lucide-react';

const ROLES = ['viewer', 'operator', 'admin', 'dispatcher'];

function parseVehicles(input: string): string[] {
  return input.split(',').map((v) => v.trim()).filter((v) => v.length > 0);
}

export default function ApiKeys() {
  const [keys, setKeys] = useState<AdminApiKey[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionMsg, setActionMsg] = useState<{ type: 'success' | 'error'; text: string } | null>(null);

  // 创建弹窗
  const [createOpen, setCreateOpen] = useState(false);
  const [newKeyId, setNewKeyId] = useState('');
  const [newRole, setNewRole] = useState('viewer');
  const [newVehicles, setNewVehicles] = useState('');
  const [newLabel, setNewLabel] = useState('');
  const [createError, setCreateError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

  // 一次性 secret 展示（注册后弹出，关闭即弃 — 仿 Devices secretModal）
  const [secretModal, setSecretModal] = useState<{ title: string; secret: string } | null>(null);
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState<string | null>(null);

  // 吊销进行中的行（防重复点击）
  const [busy, setBusy] = useState<string | null>(null);

  const fetchKeys = useCallback(async () => {
    try {
      const data = await getApiKeys();
      setKeys(data.api_keys);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch api keys');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { fetchKeys(); }, [fetchKeys]);

  const openCreate = () => {
    setCreateOpen(true);
    setNewKeyId('');
    setNewRole('viewer');
    setNewVehicles('');
    setNewLabel('');
    setCreateError(null);
  };

  const handleCreate = async () => {
    const id = newKeyId.trim();
    if (!id) { setCreateError('Key ID is required'); return; }
    if (creating) return;
    setCreating(true);
    setCreateError(null);
    try {
      const resp = await createApiKey(id, newRole, parseVehicles(newVehicles), newLabel.trim() || null);
      setCreateOpen(false);
      setSecretModal({ title: `Created ${resp.key_id}`, secret: resp.secret });
      setCopied(false);
      setCopyError(null);
      fetchKeys();
    } catch (e) {
      setCreateError(e instanceof Error ? e.message : 'Create failed');
    } finally {
      setCreating(false);
    }
  };

  const handleRevoke = async (keyId: string) => {
    if (!window.confirm(`Revoke api key ${keyId}? This cannot be undone.`)) return;
    setBusy(keyId);
    try {
      await revokeApiKey(keyId);
      setActionMsg({ type: 'success', text: `API key ${keyId} revoked` });
      fetchKeys();
    } catch (e) {
      setActionMsg({ type: 'error', text: e instanceof Error ? e.message : 'Revoke failed' });
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
  if (error && keys.length === 0) return <div className="error">{error}</div>;

  return (
    <div className="apikeys">
      <div className="apikeys-head">
        <h2 className="section-title"><KeyRound size={14} /> API Keys</h2>
        <button className="btn" onClick={openCreate}>+ Create API Key</button>
      </div>

      {actionMsg && <p className={`token-status ${actionMsg.type === 'success' ? 'saved' : 'error'}`}>{actionMsg.text}</p>}
      {error && <p className="token-status error">{error}</p>}

      {keys.length === 0 ? (
        <p className="empty">No API keys</p>
      ) : (
        <div className="apikey-list">
          <div className="apikey-row apikey-row-head">
            <span className="apikey-col-name">Key ID</span>
            <span className="apikey-col-role">Role</span>
            <span className="apikey-col-vehicles">Vehicles</span>
            <span className="apikey-col-label">Label</span>
            <span className="apikey-col-actions" />
          </div>
          {keys.map((k) => (
            <div key={k.key_id} className="apikey-row">
              <span className="apikey-col-name">{k.key_id}</span>
              <span className="apikey-col-role"><span className={`role-badge role-${k.role}`}>{k.role}</span></span>
              <span className="apikey-col-vehicles apikey-vehicles">{k.vehicles.length > 0 ? k.vehicles.join(', ') : '—'}</span>
              <span className="apikey-col-label apikey-vehicles">{k.label ?? '—'}</span>
              <span className="apikey-col-actions">
                <button className="btn-sm" disabled={busy === k.key_id} onClick={() => handleRevoke(k.key_id)}>Revoke</button>
              </span>
            </div>
          ))}
        </div>
      )}

      {createOpen && (
        <Modal title="Create API Key" onClose={() => setCreateOpen(false)}>
          <div className="form-row">
            <label className="form-label" htmlFor="apikey-id">Key ID</label>
            <input
              id="apikey-id"
              className="form-field"
              value={newKeyId}
              onChange={(e) => setNewKeyId(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') handleCreate(); }}
              placeholder="ci-bot"
            />
          </div>
          <div className="form-row">
            <label className="form-label" htmlFor="apikey-role">Role</label>
            <select id="apikey-role" className="form-field" value={newRole} onChange={(e) => setNewRole(e.target.value)}>
              {ROLES.map((r) => <option key={r} value={r}>{r}</option>)}
            </select>
          </div>
          <div className="form-row">
            <label className="form-label" htmlFor="apikey-vehicles">Vehicles</label>
            <input id="apikey-vehicles" className="form-field" value={newVehicles} onChange={(e) => setNewVehicles(e.target.value)} placeholder="vehicle-01, vehicle-02" />
          </div>
          <p className="form-hint">Vehicles: comma-separated device IDs — only meaningful for viewer/operator.</p>
          <div className="form-row">
            <label className="form-label" htmlFor="apikey-label">Label</label>
            <input id="apikey-label" className="form-field" value={newLabel} onChange={(e) => setNewLabel(e.target.value)} placeholder="optional" />
          </div>
          {createError && <p className="form-error">{createError}</p>}
          <div className="form-actions">
            <button className="btn btn-outline" onClick={() => setCreateOpen(false)}>Cancel</button>
            <button className="btn" onClick={handleCreate} disabled={creating}>{creating ? 'Creating...' : 'Create'}</button>
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
