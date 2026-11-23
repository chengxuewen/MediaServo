// T9 evidence list page (K5/K14/7.3): retained talk-spurt clips with the
// 7-day retention label, audited download, and the irreversible withdraw
// (Confirm states the consequence; admin-only via the server middleware).
import { useCallback, useEffect, useState } from 'react';
import { Button, Confirm, StatusGlyph, Table, TBody, TD, TH, THead, TR } from '@mediaservo/react';
import {
  deleteEvidence,
  fetchEvidenceBlob,
  listEvidence,
  type EvidenceItemEntry,
} from '../api/client';
import { useAuth } from '../hooks/useAuth';

const fmt = (unix: number) => new Date(unix * 1000).toLocaleString();
const kb = (bytes: number) => `${Math.max(1, Math.round(bytes / 1024))} KB`;

export default function Evidence() {
  const { isAdmin } = useAuth();
  const [items, setItems] = useState<EvidenceItemEntry[] | null>(null);
  const [retention, setRetention] = useState<number | null>(null);
  const [source, setSource] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [withdrawing, setWithdrawing] = useState<EvidenceItemEntry | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(() => {
    listEvidence()
      .then((r) => {
        setItems(r.items);
        setRetention(r.retention_secs);
        setSource(r.source);
        setError(null);
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const download = (item: EvidenceItemEntry) => {
    fetchEvidenceBlob(item.id)
      .then((blob) => {
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = `evidence-${item.id}.webm`;
        a.click();
        URL.revokeObjectURL(url);
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  };

  const withdraw = () => {
    if (!withdrawing || busy) return;
    setBusy(true);
    deleteEvidence(withdrawing.id)
      .then(() => {
        setWithdrawing(null);
        setBusy(false);
        refresh();
      })
      .catch((e: unknown) => {
        setBusy(false);
        setError(e instanceof Error ? e.message : String(e));
      });
  };

  const days = retention ? Math.round(retention / 86400) : 7;

  return (
    <section>
      <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--ms-space-3)' }}>
        <h1 style={{ margin: 0 }}>Evidence</h1>
        <StatusGlyph tone="off" shape="ring" label={`retention ${days} days`} />
        <span className="msr-talk-mode">retention {days} days · source {source || '…'}</span>
      </div>
      <p role="note" style={{ color: 'var(--ms-color-text-muted)', fontSize: 'var(--ms-font-size-sm)' }}>
        Retained talk spurts (explicit consent at declaration). Downloads and withdrawals are audited.
        Retention closes automatically after {days} days. Withdrawal is irreversible (admin only).
      </p>
      {error && <p role="alert">{error}</p>}
      {items === null && !error && <p role="status">Loading evidence…</p>}
      {items !== null && items.length === 0 && (
        <p role="status">No retained clips — use “retain 15 s” in a session to declare one.</p>
      )}
      {items !== null && items.length > 0 && (
        <Table>
          <THead>
            <TR>
              <TH>declared</TH>
              <TH>room</TH>
              <TH>actor</TH>
              <TH>label</TH>
              <TH>size</TH>
              <TH>retains until</TH>
              <TH>actions</TH>
            </TR>
          </THead>
          <TBody>
            {items.map((it) => (
              <TR key={it.id}>
                <TD>{fmt(it.ts)}</TD>
                <TD>{it.room_id}</TD>
                <TD>{it.actor}</TD>
                <TD>{it.label || '—'}</TD>
                <TD>{kb(it.size)}</TD>
                <TD>{fmt(it.retain_until)}</TD>
                <TD>
                  <span style={{ display: 'flex', gap: 'var(--ms-space-2)' }}>
                    <Button size="sm" onClick={() => download(it)}>
                      Download
                    </Button>
                    <Button
                      size="sm"
                      variant="danger"
                      disabled={!isAdmin}
                      disabledReason="withdrawal is admin-only"
                      onClick={() => setWithdrawing(it)}
                    >
                      Withdraw
                    </Button>
                  </span>
                </TD>
              </TR>
            ))}
          </TBody>
        </Table>
      )}
      {withdrawing && (
        <Confirm
          open
          title={`Withdraw ${withdrawing.room_id} clip?`}
          message="The clip is permanently erased from the server. This cannot be undone (the withdrawal itself is audited)."
          confirmLabel="Withdraw"
          tone="danger"
          onConfirm={withdraw}
          onCancel={() => setWithdrawing(null)}
        />
      )}
    </section>
  );
}
