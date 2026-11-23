// W2 (admin-prototype-restyle, B3 approved real credentials face): enrollment
// + approval + credentials lifecycle in the prototype language on the current
// list layout (drawer/config-draft/vision-switch deferred per adjudication 3).
// Reject/Restore/Re-approve have no server endpoint -> not drawn (honest).
// Revoke/reset carry semantics cards grounded in devices.yaml truth; one-time
// secrets are close-protected (discard confirm when not copied).
import { useCallback, useEffect, useState } from 'react';
import {
  Button,
  Confirm,
  Input,
  Modal,
  Table,
  TBody,
  TD,
  TH,
  THead,
  TR,
} from '@mediaservo/react';
import { getAdminDevices, registerDevice, revokeDevice, resetDeviceSecret, getPendingDevices, approvePendingDevice, getVehicleStatus } from '../api/client';
import type { AdminDevice, PendingDevice, StatusReport } from '../api/client';
import ActiveDevices from '../components/ActiveDevices';
import { useDevices } from '../hooks/useDevices';
import { fleetCounts, telemetrySummary } from '../fleet';
import { AlertTriangle, Check, Plus } from 'lucide-react';

// first-seen time → relative time (scale mirrors Vehicles fmtUptime)
function fmtAgo(firstSeenMs: number): string {
  const secs = Math.max(0, Math.floor((Date.now() - firstSeenMs) / 1000));
  if (secs < 60) return `${secs}s ago`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m ago`;
}

const monoStyle = { fontFamily: 'var(--ms-font-family-mono)' } as const;

const REVOKE_SEMANTICS = 'Revoke deletes the credential: new connections are refused immediately; established sessions live until disconnect. This cannot be undone.';
const RESET_SEMANTICS = 'Reset issues a new secret and invalidates the old one for new connections. The new value is shown once — copy it before closing.';

export default function Devices({ initialQuery = '' }: { initialQuery?: string }) {
  const [devices, setDevices] = useState<AdminDevice[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionMsg, setActionMsg] = useState<{ type: 'success' | 'error'; text: string } | null>(null);
  const [query, setQuery] = useState(initialQuery);
  // B4 fleet lens (radiogroup + roving) + health strip; fleet = devices with
  // telemetry reports (exported semantics — server has no kind field yet).
  const [lens, setLens] = useState<'all' | 'fleet'>('all');
  const [reports, setReports] = useState<{ room_id: string; report: StatusReport }[]>([]);
  const { devices: activeDevices } = useDevices();

  // register-form modal
  const [registerOpen, setRegisterOpen] = useState(false);
  const [newDeviceId, setNewDeviceId] = useState('');
  const [providedSecret, setProvidedSecret] = useState('');
  const [registerError, setRegisterError] = useState<string | null>(null);
  const [registering, setRegistering] = useState(false);

  // one-time secret display (shown after register/reset, discarded on close — never leaves this state)
  const [secretModal, setSecretModal] = useState<{ title: string; secret: string } | null>(null);
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState<string | null>(null);
  const [discardOpen, setDiscardOpen] = useState(false);

  // confirm targets (kit Confirm replaces window.confirm) + double-click guard
  const [revoking, setRevoking] = useState<string | null>(null);
  const [resetting, setResetting] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  // device-enroll: pending approval queue (manual mode; in-memory table D-E5, cleared on restart)
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
  // ponytail: 5s polling (same cadence as useDevices/Vehicles)
  useEffect(() => {
    const interval = setInterval(fetchPending, 5000);
    return () => clearInterval(interval);
  }, [fetchPending]);

  // telemetry for the fleet lens (15s cadence; stale rule lives in fleet.ts)
  useEffect(() => {
    let alive = true;
    const pull = () => getVehicleStatus().then((r) => { if (alive) setReports(r.vehicles); }).catch(() => { /* lens degrades to empty */ });
    pull();
    const interval = setInterval(pull, 15000);
    return () => { alive = false; clearInterval(interval); };
  }, []);

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

  const handleRevoke = async () => {
    if (!revoking) return;
    const deviceId = revoking;
    setBusy(deviceId);
    try {
      await revokeDevice(deviceId);
      setActionMsg({ type: 'success', text: `Device ${deviceId} revoked` });
      setRevoking(null);
      fetchDevices();
    } catch (e) {
      setActionMsg({ type: 'error', text: e instanceof Error ? e.message : 'Revoke failed' });
      setRevoking(null);
    } finally {
      setBusy(null);
    }
  };

  const handleResetSecret = async () => {
    if (!resetting) return;
    const deviceId = resetting;
    setBusy(deviceId);
    try {
      const resp = await resetDeviceSecret(deviceId);
      setSecretModal({ title: `New key for ${resp.device_id}`, secret: resp.secret });
      setCopied(false);
      setCopyError(null);
      setResetting(null);
    } catch (e) {
      setActionMsg({ type: 'error', text: e instanceof Error ? e.message : 'Reset failed' });
      setResetting(null);
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
      setActionMsg({ type: 'success', text: `${resp.device_id} approved — ${resp.note}` });
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
      // fallback: show the secret in a read-only box that can be fully selected and copied manually — do not assume clipboard is available
      setCopyError('Clipboard unavailable — select the secret manually');
    }
  };

  // close protection: an uncopied one-time secret needs an explicit discard
  const requestSecretClose = () => {
    if (copied) setSecretModal(null);
    else setDiscardOpen(true);
  };

  const q = query.trim().toLowerCase();
  const rows = devices.filter((d) => !q || d.device_id.toLowerCase().includes(q));
  const fleetSet = new Set(reports.map((r) => r.room_id));
  const lensRows = lens === 'fleet' ? rows.filter((d) => fleetSet.has(d.device_id)) : rows;
  const counts = fleetCounts(reports, new Set(activeDevices.map((d) => d.device_id)));
  const strip: { k: string; v: number; c?: string }[] = [
    { k: 'Vehicles', v: counts.n },
    { k: 'Signal lost', v: counts.x, c: 'var(--ms-color-danger)' },
    { k: 'Agent unreachable', v: counts.y, c: 'var(--ms-color-warning)' },
    { k: 'Stalled topics', v: counts.z, c: 'var(--ms-color-warning)' },
    { k: 'Down processes', v: counts.w, c: 'var(--ms-color-danger)' },
  ];

  if (loading) return <div className="loading">Loading...</div>;
  if (error && devices.length === 0) return <div className="error">{error}</div>;

  return (
    <section>
      <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--ms-space-3)' }}>
        <h1 style={{ margin: 0 }}>Devices</h1>
        <span style={{ flex: 1 }} />
        <Input
          placeholder="Search devices…"
          aria-label="Search devices"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          style={{ flex: '0 0 220px' }}
        />
        <Button variant="primary" onClick={openRegister}><Plus size={12} /> Register device</Button>
      </div>
      <p role="note" style={{ color: 'var(--ms-color-text-muted)', fontSize: 'var(--ms-font-size-sm)', margin: 'var(--ms-space-2) 0 var(--ms-space-3)' }}>
        Enrollment, approval & audit
      </p>

      {actionMsg && (
        <p role={actionMsg.type === 'success' ? 'status' : 'alert'}
          style={{ color: actionMsg.type === 'success' ? 'var(--ms-color-success)' : 'var(--ms-color-danger)', fontSize: 'var(--ms-font-size-sm)' }}>
          {actionMsg.text}
        </p>
      )}

      <h2 style={{ fontSize: 'var(--ms-font-size-sm)', fontWeight: 'var(--ms-font-weight-semibold)', margin: '0 0 var(--ms-space-2)' }}>
        Pending Devices
      </h2>
      {pendingError && (
        <p role="alert" style={{ display: 'flex', alignItems: 'center', gap: 'var(--ms-space-2)' }}>
          {pendingError} <Button size="sm" onClick={fetchPending}>Retry</Button>
        </p>
      )}
      {pending.length === 0 ? (
        <p role="status" style={{ color: 'var(--ms-color-text-muted)', fontSize: 'var(--ms-font-size-sm)' }}>No pending devices</p>
      ) : (
        <Table>
          <THead>
            <TR>
              <TH>Device</TH>
              <TH>First seen</TH>
              <TH>Name (optional)</TH>
              <TH style={{ textAlign: 'right' }}>Actions</TH>
            </TR>
          </THead>
          <TBody>
            {pending.map((p) => (
              <TR key={p.device_id}>
                <TD><b style={monoStyle}>{p.device_id}</b></TD>
                <TD>{fmtAgo(p.first_seen_ms)}</TD>
                <TD>
                  <Input
                    aria-label={`Name for ${p.device_id}`}
                    placeholder="Name (optional)"
                    value={pendingNames[p.device_id] ?? ''}
                    onChange={(e) => setPendingNames((prev) => ({ ...prev, [p.device_id]: e.target.value }))}
                  />
                </TD>
                <TD>
                  <span style={{ display: 'flex', justifyContent: 'flex-end' }}>
                    <Button size="sm" variant="primary" disabled={pendingBusy === p.device_id} disabledReason="Approving…" onClick={() => handleApprove(p.device_id)}>
                      Approve
                    </Button>
                  </span>
                </TD>
              </TR>
            ))}
          </TBody>
        </Table>
      )}

      {/* B4 lens: radiogroup + roving tabindex; fleet strip shows the five
          health counts with the unreachable-supersede rule (fleet.ts). */}
      <div role="radiogroup" aria-label="Device lens" style={{ display: 'flex', gap: 'var(--ms-space-1)', margin: 'var(--ms-space-4) 0 var(--ms-space-2)' }}>
        {(['all', 'fleet'] as const).map((l) => (
          <button
            key={l}
            role="radio"
            aria-checked={lens === l}
            tabIndex={lens === l ? 0 : -1}
            onClick={() => setLens(l)}
            onKeyDown={(e) => {
              if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
                e.preventDefault();
                setLens(lens === 'all' ? 'fleet' : 'all');
              }
            }}
            style={{
              height: 'var(--ms-control-h-md)',
              padding: '0 var(--ms-space-3)',
              border: '1px solid var(--ms-color-border)',
              borderRadius: 'var(--ms-radius-sm)',
              background: lens === l ? 'var(--ms-color-primary)' : 'var(--ms-color-surface)',
              color: lens === l ? 'var(--ms-color-text-inverse)' : 'var(--ms-color-text-secondary)',
              fontSize: 'var(--ms-font-size-xs)',
              cursor: 'pointer',
            }}
          >
            {l === 'all' ? 'All devices' : 'Fleet'}
          </button>
        ))}
      </div>
      {lens === 'fleet' && (
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(5, minmax(0, 1fr))', gap: 'var(--ms-space-3)', marginBottom: 'var(--ms-space-3)' }}>
          {strip.map((s) => (
            <div key={s.k} style={{ background: 'var(--ms-color-surface)', border: '1px solid var(--ms-color-border)', borderRadius: 'var(--ms-radius-md)', padding: 'var(--ms-space-2) var(--ms-space-3)' }}>
              <div style={{ fontSize: 'var(--ms-font-size-xs)', color: 'var(--ms-color-text-tertiary)' }}>{s.k}</div>
              <div style={{ fontSize: 'var(--ms-font-size-md)', fontWeight: 'var(--ms-font-weight-semibold)', color: s.c ?? 'var(--ms-color-text)' }}>{s.v}</div>
            </div>
          ))}
        </div>
      )}

      <h2 style={{ fontSize: 'var(--ms-font-size-sm)', fontWeight: 'var(--ms-font-weight-semibold)', margin: '0 0 var(--ms-space-2)' }}>
        Registered Devices
      </h2>
      {error && <p role="alert">{error}</p>}
      {devices.length === 0 ? (
        <p role="status" style={{ color: 'var(--ms-color-text-muted)', fontSize: 'var(--ms-font-size-sm)' }}>No devices registered</p>
      ) : (
        <Table>
          <THead>
            <TR>
              <TH>Device</TH>
              {lens === 'fleet' && <TH>Telemetry</TH>}
              <TH style={{ textAlign: 'right' }}>Actions</TH>
            </TR>
          </THead>
          <TBody>
            {lensRows.map((d) => {
              const t = telemetrySummary(reports, d.device_id);
              return (
                <TR key={d.device_id}>
                  <TD><b style={monoStyle}>{d.device_id}</b></TD>
                  {lens === 'fleet' && (
                    <TD>
                      {t ? (
                        <span style={{ fontSize: 'var(--ms-font-size-xs)', color: t.signal === 'ok' ? 'var(--ms-color-text-secondary)' : 'var(--ms-color-danger)' }}>
                          {t.signal === 'ok' ? 'signal ok' : 'signal lost'} · {t.stalled} stalled · {t.down} down
                        </span>
                      ) : (
                        <span style={{ fontSize: 'var(--ms-font-size-xs)', color: 'var(--ms-color-text-muted)' }}>no telemetry</span>
                      )}
                    </TD>
                  )}
                  <TD>
                    <span style={{ display: 'flex', gap: 'var(--ms-space-2)', justifyContent: 'flex-end' }}>
                      <Button size="sm" disabled={busy === d.device_id} disabledReason="Working…" onClick={() => setResetting(d.device_id)}>
                        Reset secret
                      </Button>
                      <Button size="sm" variant="danger" disabled={busy === d.device_id} disabledReason="Working…" onClick={() => setRevoking(d.device_id)}>
                        Revoke
                      </Button>
                    </span>
                  </TD>
                </TR>
              );
            })}
            {lensRows.length === 0 && (
              <TR>
                <TD colSpan={lens === 'fleet' ? 3 : 2}>
                  <span style={{ color: 'var(--ms-color-text-muted)' }}>
                    {lens === 'fleet' ? 'No fleet devices with telemetry.' : 'No devices match.'}
                  </span>
                </TD>
              </TR>
            )}
          </TBody>
        </Table>
      )}

      {revoking && (
        <Confirm
          open
          title={`Revoke ${revoking}?`}
          message={REVOKE_SEMANTICS}
          confirmLabel="Revoke"
          tone="danger"
          onConfirm={handleRevoke}
          onCancel={() => setRevoking(null)}
        />
      )}

      {resetting && (
        <Confirm
          open
          title={`Reset secret for ${resetting}?`}
          message={RESET_SEMANTICS}
          confirmLabel="Reset"
          tone="danger"
          onConfirm={handleResetSecret}
          onCancel={() => setResetting(null)}
        />
      )}

      {registerOpen && (
        <Modal
          open
          title="Register device"
          onClose={() => setRegisterOpen(false)}
          actions={
            <>
              <Button onClick={() => setRegisterOpen(false)}>Cancel</Button>
              <Button variant="primary" disabled={registering} disabledReason="Registering…" onClick={handleRegister}>
                {registering ? 'Registering...' : 'Register'}
              </Button>
            </>
          }
        >
          <Input
            label="Device ID"
            id="device-id"
            value={newDeviceId}
            onChange={(e) => setNewDeviceId(e.target.value)}
            placeholder="ms-0a1b2c3d4e5f"
          />
          <Input
            label="Secret"
            id="device-secret"
            type="password"
            value={providedSecret}
            onChange={(e) => setProvidedSecret(e.target.value)}
            placeholder="leave blank for auto-generate"
            hint="Optional — configure the host first, then register with the same secret; blank = server generates."
          />
          {registerError && <p role="alert">{registerError}</p>}
        </Modal>
      )}

      {secretModal && (
        <Modal
          open
          title={secretModal.title}
          onClose={requestSecretClose}
          actions={
            <>
              <Button variant="primary" onClick={handleCopy}>{copied ? <><Check size={12} /> Copied</> : 'Copy'}</Button>
              <Button onClick={requestSecretClose}>Done</Button>
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
            aria-label="Device secret"
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

      {discardOpen && (
        <Confirm
          open
          title="Discard the one-time secret?"
          message="It was not copied. Closing loses it forever — reset the secret later to issue a new one."
          confirmLabel="Discard"
          tone="danger"
          onConfirm={() => { setDiscardOpen(false); setSecretModal(null); }}
          onCancel={() => setDiscardOpen(false)}
        />
      )}

      {/* W1 relocation: the overview is a health summary; the live device
          tree + playback wall lives here (behavior unchanged). */}
      <ActiveDevices />
    </section>
  );
}