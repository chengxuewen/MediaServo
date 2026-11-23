// W6 (admin-prototype-restyle, B1 approved real-data face): Accounts in the
// prototype language — head + search + kit Table (Username/Role/Device scope/
// actions) + create/edit modals + kit Confirm. The AccessBase provider layer
// (delegated/degraded/break-glass) is deferred per adjudication 1 and NOT
// implemented here. Capability index is an honest read-only map: real faces
// link, absent faces say so (audit page pending B6; SSO/LDAP not in build).
// Scope semantics fixed to server truth (roles.rs can_pull): viewer/operator
// empty allowlist = NO devices (the old "empty = all" hint was wrong).
import { useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import {
  Button,
  Combobox,
  Confirm,
  Field,
  Input,
  Modal,
  Table,
  TBody,
  TD,
  TH,
  THead,
  TR,
} from '@mediaservo/react';
import { getAdminAccounts, createAccount, updateAccount, deleteAccount, getUsername } from '../api/client';
import type { AdminAccount, AccountRole } from '../api/client';
import { Plus } from 'lucide-react';

const ROLES: AccountRole[] = ['viewer', 'operator', 'admin', 'dispatcher'];
const ROLE_OPTIONS = ROLES.map((r) => ({ value: r, label: r }));

function parseVehicles(input: string): string[] {
  return input.split(',').map((v) => v.trim()).filter((v) => v.length > 0);
}

const chipStyle = {
  display: 'inline-block',
  padding: '0 var(--ms-space-2)',
  border: '1px solid var(--ms-color-border)',
  borderRadius: 'var(--ms-radius-full)',
  fontSize: 'var(--ms-font-size-xs)',
  color: 'var(--ms-color-text-secondary)',
} as const;

const monoStyle = { fontFamily: 'var(--ms-font-family-mono)' } as const;

const SCOPE_HINT = 'Comma-separated device IDs. viewer/operator: empty = no devices; admin/dispatcher: any device.';

// Honest capability index (B1): where each capability actually lives today.
const CAP_INDEX: { cap: string; where: string; tab?: string }[] = [
  { cap: 'Accounts', where: 'this page' },
  { cap: 'Roles & device scope', where: 'this page (role + allowlist)' },
  { cap: 'Authentication', where: 'server login; no separate page' },
  { cap: 'PSK', where: 'Settings', tab: 'settings' },
  { cap: 'Device credentials', where: 'Devices + Keys', tab: 'devices' },
  { cap: 'Authorization policies', where: 'server role middleware; no page' },
  { cap: 'Security audit', where: 'API live; page pending board B6' },
  { cap: 'SSO / MFA', where: 'not in this build' },
  { cap: 'LDAP / AD federation', where: 'not in this build' },
];

export default function Accounts() {
  const navigate = useNavigate();
  const [accounts, setAccounts] = useState<AdminAccount[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionMsg, setActionMsg] = useState<{ type: 'success' | 'error'; text: string } | null>(null);
  const [query, setQuery] = useState('');
  const currentUsername = getUsername();

  // create-account modal
  const [createOpen, setCreateOpen] = useState(false);
  const [createUsername, setCreateUsername] = useState('');
  const [createPassword, setCreatePassword] = useState('');
  const [createRole, setCreateRole] = useState<AccountRole>('viewer');
  const [createVehicles, setCreateVehicles] = useState('');
  const [createError, setCreateError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  // edit-account modal
  const [editing, setEditing] = useState<AdminAccount | null>(null);
  const [editRole, setEditRole] = useState<AccountRole>('viewer');
  const [editVehicles, setEditVehicles] = useState('');
  const [editPassword, setEditPassword] = useState('');
  const [editError, setEditError] = useState<string | null>(null);

  // delete confirm (kit Confirm replaces window.confirm)
  const [deleting, setDeleting] = useState<AdminAccount | null>(null);

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

  const handleDelete = async () => {
    if (!deleting) return;
    const username = deleting.username;
    try {
      const resp = await deleteAccount(username);
      setActionMsg({ type: 'success', text: `Account ${resp.deleted} deleted` });
      setDeleting(null);
      fetchAccounts();
    } catch (e) {
      setActionMsg({ type: 'error', text: e instanceof Error ? e.message : 'Delete failed' });
      setDeleting(null);
    }
  };

  const q = query.trim().toLowerCase();
  const rows = accounts.filter((a) => !q || `${a.username}${a.role}${a.vehicles.join(',')}`.toLowerCase().includes(q));

  return (
    <section>
      <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--ms-space-3)' }}>
        <h1 style={{ margin: 0 }}>Accounts</h1>
        <span style={{ flex: 1 }} />
        <Input
          placeholder="Search accounts…"
          aria-label="Search accounts"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          style={{ flex: '0 0 220px' }}
        />
        <Button variant="primary" onClick={openCreate}><Plus size={12} /> New account</Button>
      </div>
      <p role="note" style={{ color: 'var(--ms-color-text-muted)', fontSize: 'var(--ms-font-size-sm)', margin: 'var(--ms-space-2) 0 var(--ms-space-3)' }}>
        Account management · roles & device scope
      </p>

      {actionMsg && (
        <p role={actionMsg.type === 'success' ? 'status' : 'alert'}
          style={{ color: actionMsg.type === 'success' ? 'var(--ms-color-success)' : 'var(--ms-color-danger)', fontSize: 'var(--ms-font-size-sm)' }}>
          {actionMsg.text}
        </p>
      )}
      {error && (
        <p role="alert" style={{ display: 'flex', alignItems: 'center', gap: 'var(--ms-space-2)' }}>
          {error} <Button size="sm" onClick={fetchAccounts}>Retry</Button>
        </p>
      )}
      {loading && <p role="status" style={{ marginTop: 'var(--ms-space-3)' }}>Loading accounts…</p>}

      {!loading && accounts.length === 0 && !error && (
        <p role="status" style={{ marginTop: 'var(--ms-space-3)' }}>No accounts yet — create the first cockpit account.</p>
      )}

      {!loading && accounts.length > 0 && (
        <Table>
          <THead>
            <TR>
              <TH>Username</TH>
              <TH>Role</TH>
              <TH>Device scope</TH>
              <TH style={{ textAlign: 'right' }}>Actions</TH>
            </TR>
          </THead>
          <TBody>
            {rows.map((acc) => (
              <TR key={acc.username}>
                <TD><b style={monoStyle}>{acc.username}</b></TD>
                <TD><span style={chipStyle}>{acc.role}</span></TD>
                <TD>
                  {acc.role === 'admin' || acc.role === 'dispatcher' ? (
                    <span style={{ color: 'var(--ms-color-text-tertiary)' }}>— (all devices)</span>
                  ) : acc.vehicles.length > 0 ? (
                    <span style={{ display: 'flex', gap: 'var(--ms-space-1)', flexWrap: 'wrap' }}>
                      {acc.vehicles.map((v) => <span key={v} style={chipStyle}>{v}</span>)}
                    </span>
                  ) : (
                    <span style={chipStyle}>no devices</span>
                  )}
                </TD>
                <TD>
                  <span style={{ display: 'flex', gap: 'var(--ms-space-2)', justifyContent: 'flex-end' }}>
                    <Button size="sm" onClick={() => openEdit(acc)}>Edit</Button>
                    <Button
                      size="sm"
                      variant="danger"
                      disabled={acc.username === currentUsername}
                      disabledReason="Cannot delete the account you are logged in as"
                      onClick={() => setDeleting(acc)}
                    >
                      Delete
                    </Button>
                  </span>
                </TD>
              </TR>
            ))}
            {rows.length === 0 && (
              <TR>
                <TD colSpan={4}>
                  <span style={{ color: 'var(--ms-color-text-muted)' }}>No accounts match.</span>
                </TD>
              </TR>
            )}
          </TBody>
        </Table>
      )}

      <h2 style={{ fontSize: 'var(--ms-font-size-sm)', fontWeight: 'var(--ms-font-weight-semibold)', margin: 'var(--ms-space-4) 0 var(--ms-space-1)' }}>
        Capability index
      </h2>
      <p role="note" style={{ color: 'var(--ms-color-text-tertiary)', fontSize: 'var(--ms-font-size-xs)', margin: '0 0 var(--ms-space-2)' }}>
        Read-only map of where each access capability lives today. Absent faces are labelled, not simulated.
      </p>
      <Table>
        <THead>
          <TR>
            <TH>Capability</TH>
            <TH>Where it lives</TH>
          </TR>
        </THead>
        <TBody>
          {CAP_INDEX.map((c) => (
            <TR key={c.cap}>
              <TD>{c.cap}</TD>
              <TD>
                {c.tab ? (
                  <Button size="sm" variant="ghost" onClick={() => navigate('/admin', { state: { tab: c.tab } })}>
                    {c.where}
                  </Button>
                ) : (
                  <span style={{ color: 'var(--ms-color-text-tertiary)' }}>{c.where}</span>
                )}
              </TD>
            </TR>
          ))}
        </TBody>
      </Table>

      {deleting && (
        <Confirm
          open
          title={`Delete ${deleting.username}?`}
          message="The account loses access immediately. This cannot be undone."
          confirmLabel="Delete"
          tone="danger"
          onConfirm={handleDelete}
          onCancel={() => setDeleting(null)}
        />
      )}

      {createOpen && (
        <Modal
          open
          title="New account"
          onClose={() => setCreateOpen(false)}
          actions={
            <>
              <Button onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button variant="primary" disabled={saving} disabledReason="Saving…" onClick={handleCreate}>
                {saving ? 'Creating...' : 'Create'}
              </Button>
            </>
          }
        >
          <Input label="Username" id="acc-username" value={createUsername} onChange={(e) => setCreateUsername(e.target.value)} placeholder="operator-01" />
          <Input label="Password" id="acc-password" type="password" value={createPassword} onChange={(e) => setCreatePassword(e.target.value)} />
          <Field label="Role" htmlFor="acc-role">
            <Combobox id="acc-role" options={ROLE_OPTIONS} value={createRole} onChange={(v) => setCreateRole(v as AccountRole)} aria-label="Role" />
          </Field>
          <Input label="Vehicles" id="acc-vehicles" value={createVehicles} onChange={(e) => setCreateVehicles(e.target.value)} placeholder="vehicle-01, vehicle-02" hint={SCOPE_HINT} />
          {createError && <p role="alert">{createError}</p>}
        </Modal>
      )}

      {editing && (
        <Modal
          open
          title={`Edit ${editing.username}`}
          onClose={() => setEditing(null)}
          actions={
            <>
              <Button onClick={() => setEditing(null)}>Cancel</Button>
              <Button variant="primary" disabled={saving} disabledReason="Saving…" onClick={handleUpdate}>
                {saving ? 'Saving...' : 'Save'}
              </Button>
            </>
          }
        >
          <Field label="Role" htmlFor="edit-role">
            <Combobox id="edit-role" options={ROLE_OPTIONS} value={editRole} onChange={(v) => setEditRole(v as AccountRole)} aria-label="Role" />
          </Field>
          <Input label="Vehicles" id="edit-vehicles" value={editVehicles} onChange={(e) => setEditVehicles(e.target.value)} placeholder="vehicle-01, vehicle-02" hint={SCOPE_HINT} />
          <Input label="New password" id="edit-password" type="password" value={editPassword} onChange={(e) => setEditPassword(e.target.value)} placeholder="Leave blank to keep current" />
          {editError && <p role="alert">{editError}</p>}
        </Modal>
      )}
    </section>
  );
}