import { useCallback, useEffect, useState } from 'react';
import { getAdminAccounts, createAccount, updateAccount, deleteAccount, getUsername } from '../api/client';
import type { AdminAccount, AccountRole } from '../api/client';
import Modal from '../components/Modal';
import './Accounts.css';

const ROLES: AccountRole[] = ['viewer', 'operator', 'admin', 'dispatcher'];

function parseVehicles(input: string): string[] {
  return input.split(',').map((v) => v.trim()).filter((v) => v.length > 0);
}

export default function Accounts() {
  const [accounts, setAccounts] = useState<AdminAccount[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionMsg, setActionMsg] = useState<{ type: 'success' | 'error'; text: string } | null>(null);
  const currentUsername = getUsername();

  // 创建账号弹窗
  const [createOpen, setCreateOpen] = useState(false);
  const [createUsername, setCreateUsername] = useState('');
  const [createPassword, setCreatePassword] = useState('');
  const [createRole, setCreateRole] = useState<AccountRole>('viewer');
  const [createVehicles, setCreateVehicles] = useState('');
  const [createError, setCreateError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // 编辑账号弹窗
  const [editing, setEditing] = useState<AdminAccount | null>(null);
  const [editRole, setEditRole] = useState<AccountRole>('viewer');
  const [editVehicles, setEditVehicles] = useState('');
  const [editPassword, setEditPassword] = useState('');
  const [editError, setEditError] = useState<string | null>(null);

  const fetchAccounts = useCallback(async () => {
    try {
      const data = await getAdminAccounts();
      setAccounts(data.accounts);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch accounts');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { fetchAccounts(); }, [fetchAccounts]);

  const openCreate = () => {
    setCreateOpen(true);
    setCreateUsername('');
    setCreatePassword('');
    setCreateRole('viewer');
    setCreateVehicles('');
    setCreateError(null);
  };

  const handleCreate = async () => {
    const username = createUsername.trim();
    if (!username || !createPassword) { setCreateError('Username and password are required'); return; }
    if (saving) return;
    setSaving(true);
    setCreateError(null);
    try {
      const resp = await createAccount(username, createPassword, createRole, parseVehicles(createVehicles));
      setCreateOpen(false);
      setActionMsg({ type: 'success', text: `Account ${resp.created} created` });
      fetchAccounts();
    } catch (e) {
      setCreateError(e instanceof Error ? e.message : 'Create failed');
    } finally {
      setSaving(false);
    }
  };

  const openEdit = (acc: AdminAccount) => {
    setEditing(acc);
    setEditRole(acc.role as AccountRole);
    setEditVehicles(acc.vehicles.join(', '));
    setEditPassword('');
    setEditError(null);
  };

  const handleUpdate = async () => {
    if (!editing || saving) return;
    setSaving(true);
    setEditError(null);
    try {
      const patch: { role?: string; vehicles?: string[]; new_password?: string } = {
        role: editRole,
        vehicles: parseVehicles(editVehicles),
      };
      if (editPassword.trim()) patch.new_password = editPassword;
      const resp = await updateAccount(editing.username, patch);
      setEditing(null);
      setActionMsg({ type: 'success', text: `Account ${resp.updated} updated` });
      fetchAccounts();
    } catch (e) {
      setEditError(e instanceof Error ? e.message : 'Update failed');
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (username: string) => {
    if (!window.confirm(`Delete account ${username}?`)) return;
    try {
      const resp = await deleteAccount(username);
      setActionMsg({ type: 'success', text: `Account ${resp.deleted} deleted` });
      fetchAccounts();
    } catch (e) {
      setActionMsg({ type: 'error', text: e instanceof Error ? e.message : 'Delete failed' });
    }
  };

  if (loading) return <div className="loading">Loading...</div>;
  if (error && accounts.length === 0) return <div className="error">{error}</div>;

  return (
    <div className="accounts">
      <div className="accounts-head">
        <h2 className="section-title">Accounts</h2>
        <button className="btn" onClick={openCreate}>+ Create Account</button>
      </div>

      {actionMsg && <p className={`token-status ${actionMsg.type === 'success' ? 'saved' : 'error'}`}>{actionMsg.text}</p>}
      {error && <p className="token-status error">{error}</p>}

      {accounts.length === 0 ? (
        <p className="empty">No accounts</p>
      ) : (
        <div className="account-list">
          <div className="account-row account-row-head">
            <span className="account-col-name">Username</span>
            <span className="account-col-role">Role</span>
            <span className="account-col-vehicles">Vehicles</span>
            <span className="account-col-actions" />
          </div>
          {accounts.map((acc) => (
            <div key={acc.username} className="account-row">
              <span className="account-col-name">{acc.username}</span>
              <span className="account-col-role"><span className={`role-badge role-${acc.role}`}>{acc.role}</span></span>
              <span className="account-col-vehicles account-vehicles">{acc.vehicles.length > 0 ? acc.vehicles.join(', ') : '—'}</span>
              <span className="account-col-actions">
                <button className="btn-edit" onClick={() => openEdit(acc)}>Edit</button>
                <button
                  className="btn-sm"
                  disabled={acc.username === currentUsername}
                  title={acc.username === currentUsername ? 'Cannot delete the account you are logged in as' : undefined}
                  onClick={() => handleDelete(acc.username)}
                >
                  Delete
                </button>
              </span>
            </div>
          ))}
        </div>
      )}

      {createOpen && (
        <Modal title="Create Account" onClose={() => setCreateOpen(false)}>
          <div className="form-row">
            <label className="form-label" htmlFor="acc-username">Username</label>
            <input id="acc-username" className="form-field" value={createUsername} onChange={(e) => setCreateUsername(e.target.value)} placeholder="operator-01" />
          </div>
          <div className="form-row">
            <label className="form-label" htmlFor="acc-password">Password</label>
            <input id="acc-password" className="form-field" type="password" value={createPassword} onChange={(e) => setCreatePassword(e.target.value)} />
          </div>
          <div className="form-row">
            <label className="form-label" htmlFor="acc-role">Role</label>
            <select id="acc-role" className="form-field" value={createRole} onChange={(e) => setCreateRole(e.target.value as AccountRole)}>
              {ROLES.map((r) => <option key={r} value={r}>{r}</option>)}
            </select>
          </div>
          <div className="form-row">
            <label className="form-label" htmlFor="acc-vehicles">Vehicles</label>
            <input id="acc-vehicles" className="form-field" value={createVehicles} onChange={(e) => setCreateVehicles(e.target.value)} placeholder="vehicle-01, vehicle-02" />
          </div>
          <p className="form-hint">Vehicles: comma-separated device IDs (empty = all).</p>
          {createError && <p className="form-error">{createError}</p>}
          <div className="form-actions">
            <button className="btn btn-outline" onClick={() => setCreateOpen(false)}>Cancel</button>
            <button className="btn" onClick={handleCreate} disabled={saving}>{saving ? 'Creating...' : 'Create'}</button>
          </div>
        </Modal>
      )}

      {editing && (
        <Modal title={`Edit ${editing.username}`} onClose={() => setEditing(null)}>
          <div className="form-row">
            <label className="form-label" htmlFor="edit-role">Role</label>
            <select id="edit-role" className="form-field" value={editRole} onChange={(e) => setEditRole(e.target.value as AccountRole)}>
              {ROLES.map((r) => <option key={r} value={r}>{r}</option>)}
            </select>
          </div>
          <div className="form-row">
            <label className="form-label" htmlFor="edit-vehicles">Vehicles</label>
            <input id="edit-vehicles" className="form-field" value={editVehicles} onChange={(e) => setEditVehicles(e.target.value)} placeholder="vehicle-01, vehicle-02" />
          </div>
          <div className="form-row">
            <label className="form-label" htmlFor="edit-password">New Password</label>
            <input id="edit-password" className="form-field" type="password" value={editPassword} onChange={(e) => setEditPassword(e.target.value)} placeholder="Leave blank to keep current" />
          </div>
          {editError && <p className="form-error">{editError}</p>}
          <div className="form-actions">
            <button className="btn btn-outline" onClick={() => setEditing(null)}>Cancel</button>
            <button className="btn" onClick={handleUpdate} disabled={saving}>{saving ? 'Saving...' : 'Save'}</button>
          </div>
        </Modal>
      )}
    </div>
  );
}