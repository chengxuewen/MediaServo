// W3 (admin-prototype-restyle): Keys surface in the prototype language —
// page head + search toolbar + kit Table (Key / Role / Device scope / Label /
// actions). PSK card + admin bootstrap token partition belong to the B2 board
// (W4 Settings, H4-gated) and stay out of this page; the backend has no
// expires field, so no Expires column is fabricated.
import { useCallback, useEffect, useState, type FormEvent } from 'react';
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
import { getApiKeys, createApiKey, revokeApiKey } from '../api/client';
import type { AdminApiKey } from '../api/client';
import { AlertTriangle, Check, Plus } from 'lucide-react';

const ROLES = ['viewer', 'operator', 'admin', 'dispatcher'];
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

export default function ApiKeys() {
  const [keys, setKeys] = useState<AdminApiKey[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionMsg, setActionMsg] = useState<{ type: 'success' | 'error'; text: string } | null>(null);
  const [query, setQuery] = useState('');

  // create modal
  const [createOpen, setCreateOpen] = useState(false);
  const [newKeyId, setNewKeyId] = useState('');
  const [newRole, setNewRole] = useState('viewer');
  const [newVehicles, setNewVehicles] = useState('');
  const [newLabel, setNewLabel] = useState('');
  const [createError, setCreateError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

  // one-time secret display (shown after creation, discarded on close — mirrors the Devices secretModal)
  const [secretModal, setSecretModal] = useState<{ title: string; secret: string } | null>(null);
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState<string | null>(null);

  // revoke confirm (kit Confirm replaces window.confirm) + double-click guard
  const [revoking, setRevoking] = useState<AdminApiKey | null>(null);
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

  const handleCreate = async (e?: FormEvent) => {
    e?.preventDefault();
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
    } catch (err) {
      setCreateError(err instanceof Error ? err.message : 'Create failed');
    } finally {
      setCreating(false);
    }
  };

  const handleRevoke = async () => {
    if (!revoking) return;
    const keyId = revoking.key_id;
    setBusy(keyId);
    try {
      await revokeApiKey(keyId);
      setActionMsg({ type: 'success', text: `API key ${keyId} revoked` });
      setRevoking(null);
      fetchKeys();
    } catch (e) {
      setActionMsg({ type: 'error', text: e instanceof Error ? e.message : 'Revoke failed' });
      setRevoking(null);
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
      // fallback: show the secret in a read-only box that can be fully selected and copied manually — do not assume clipboard is available
      setCopyError('Clipboard unavailable — select the secret manually');
    }
  };

  const q = query.trim().toLowerCase();
  const rows = keys.filter((k) => !q || `${k.key_id}${k.label ?? ''}${k.role}`.toLowerCase().includes(q));

  return (
    <section>
      <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--ms-space-3)' }}>
        <h1 style={{ margin: 0 }}>Keys</h1>
        <span style={{ flex: 1 }} />
        <Input
          placeholder="Search keys…"
          aria-label="Search keys"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          style={{ flex: '0 0 220px' }}
        />
        <Button variant="primary" onClick={openCreate}><Plus size={12} /> New key</Button>
      </div>
      <p role="note" style={{ color: 'var(--ms-color-text-muted)', fontSize: 'var(--ms-font-size-sm)', margin: 'var(--ms-space-2) 0 var(--ms-space-3)' }}>
        API keys for CI and agents. The pre-shared key (device/agent auth) lives in Settings.
      </p>

      {actionMsg && (
        <p role={actionMsg.type === 'success' ? 'status' : 'alert'}
          style={{ color: actionMsg.type === 'success' ? 'var(--ms-color-success)' : 'var(--ms-color-danger)', fontSize: 'var(--ms-font-size-sm)' }}>
          {actionMsg.text}
        </p>
      )}
      {error && <p role="alert">{error}</p>}
      {loading && <p role="status" style={{ marginTop: 'var(--ms-space-3)' }}>Loading keys…</p>}

      {!loading && keys.length === 0 && (
        <p role="status" style={{ marginTop: 'var(--ms-space-3)' }}>No API keys yet — create one for CI or agents.</p>
      )}

      {!loading && keys.length > 0 && (
        <Table>
          <THead>
            <TR>
              <TH>Key</TH>
              <TH>Role</TH>
              <TH>Device scope</TH>
              <TH>Label</TH>
              <TH style={{ textAlign: 'right' }}>Actions</TH>
            </TR>
          </THead>
          <TBody>
            {rows.map((k) => (
              <TR key={k.key_id}>
                <TD><b style={monoStyle}>{k.key_id}</b></TD>
                <TD><span style={chipStyle}>{k.role}</span></TD>
                <TD>
                  {k.role === 'admin' || k.role === 'dispatcher' ? (
                    <span style={{ color: 'var(--ms-color-text-tertiary)' }}>— (all devices)</span>
                  ) : k.vehicles.length > 0 ? (
                    <span style={{ display: 'flex', gap: 'var(--ms-space-1)', flexWrap: 'wrap' }}>
                      {k.vehicles.map((v) => <span key={v} style={chipStyle}>{v}</span>)}
                    </span>
                  ) : (
                    <span style={chipStyle}>no devices</span>
                  )}
                </TD>
                <TD>{k.label ?? '—'}</TD>
                <TD>
                  <span style={{ display: 'flex', gap: 'var(--ms-space-2)', justifyContent: 'flex-end' }}>
                    <Button size="sm" variant="danger" disabled={busy === k.key_id} onClick={() => setRevoking(k)}>
                      Revoke
                    </Button>
                  </span>
                </TD>
              </TR>
            ))}
            {rows.length === 0 && (
              <TR>
                <TD colSpan={5}>
                  <span style={{ color: 'var(--ms-color-text-muted)' }}>No keys match.</span>
                </TD>
              </TR>
            )}
          </TBody>
        </Table>
      )}

      {revoking && (
        <Confirm
          open
          title={`Revoke ${revoking.key_id}?`}
          message="This cannot be undone. Clients using the key lose access immediately."
          confirmLabel="Revoke"
          tone="danger"
          onConfirm={handleRevoke}
          onCancel={() => setRevoking(null)}
        />
      )}

      {createOpen && (
        <Modal
          open
          title="New key"
          onClose={() => setCreateOpen(false)}
          actions={
            <>
              <Button onClick={() => setCreateOpen(false)}>Cancel</Button>
              <Button variant="primary" disabled={creating} onClick={() => handleCreate()}>
                {creating ? 'Creating...' : 'Create'}
              </Button>
            </>
          }
        >
          <form onSubmit={handleCreate}>
            <Input
              label="Key ID"
              id="apikey-id"
              value={newKeyId}
              onChange={(e) => setNewKeyId(e.target.value)}
              placeholder="ci-bot"
              error={createError === 'Key ID is required' ? createError : undefined}
            />
            <Field label="Role" htmlFor="apikey-role">
              <Combobox
                id="apikey-role"
                options={ROLE_OPTIONS}
                value={newRole}
                onChange={(v) => setNewRole(v)}
                aria-label="Role"
              />
            </Field>
            <Input
              label="Vehicles"
              id="apikey-vehicles"
              value={newVehicles}
              onChange={(e) => setNewVehicles(e.target.value)}
              placeholder="vehicle-01, vehicle-02"
              hint="Comma-separated device IDs — only meaningful for viewer/operator."
            />
            <Input
              label="Label"
              id="apikey-label"
              value={newLabel}
              onChange={(e) => setNewLabel(e.target.value)}
              placeholder="optional"
            />
            {createError && createError !== 'Key ID is required' && <p role="alert">{createError}</p>}
          </form>
        </Modal>
      )}

      {secretModal && (
        <Modal
          open
          title={secretModal.title}
          onClose={() => setSecretModal(null)}
          actions={
            <>
              <Button variant="primary" onClick={handleCopy}>{copied ? <><Check size={12} /> Copied</> : 'Copy'}</Button>
              <Button onClick={() => setSecretModal(null)}>Done</Button>
            </>
          }
        >
          <p role="alert" style={{ color: 'var(--ms-color-warning)', fontSize: 'var(--ms-font-size-sm)' }}>
            <AlertTriangle size={14} /> This secret is shown only once — save it now.
          </p>
          <textarea
            readOnly
            value={secretModal.secret}
            rows={3}
            onFocus={(e) => e.target.select()}
            aria-label="API key secret"
            style={{
              width: '100%',
              ...monoStyle,
              fontSize: 'var(--ms-font-size-sm)',
              padding: 'var(--ms-space-2) var(--ms-space-3)',
              border: '1px solid var(--ms-color-border)',
              borderRadius: 'var(--ms-radius-sm)',
              resize: 'vertical',
              background: 'var(--ms-color-surface-subtle)',
              color: 'var(--ms-color-text)',
            }}
          />
          {copyError && <p role="alert">{copyError}</p>}
        </Modal>
      )}
    </section>
  );
}