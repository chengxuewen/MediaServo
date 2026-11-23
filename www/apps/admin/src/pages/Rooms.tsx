// T10 managed-rooms admin page: talk-room config CRUD (id/kind/video +
// per-room floor limit overlays from SB-2). Uses the W0 component kit
// (Modal/Confirm/Combobox live use; confirmDlg instead of window.confirm).
// Dispatchers read; mutations are admin-only (guarded with a reason).
// NOTE: deleting a room removes the config record only -- live sessions
// continue until peers leave (cascade kick is a registered follow-up).
import { useCallback, useEffect, useState, type FormEvent } from 'react';
import {
  Button,
  Combobox,
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
import {
  createManagedRoom,
  deleteManagedRoom,
  listManagedRooms,
  updateManagedRoom,
  type ManagedRoomEntry,
} from '../api/client';
import { useAuth } from '../hooks/useAuth';

const KIND_OPTIONS = ['talk', 'meet', 'desk', 'view', 'ctrl', 'stream'].map((k) => ({ value: k, label: k }));
const VIDEO_OPTIONS = [
  { value: 'inherit', label: 'inherit' },
  { value: 'on', label: 'on' },
  { value: 'off', label: 'off' },
];

interface Draft {
  id: string;
  kind: string;
  video: 'inherit' | 'on' | 'off';
  maxPeers: string;
  maxQueue: string;
  holdSecs: string;
}

const emptyDraft = (): Draft => ({
  id: '',
  kind: 'talk',
  video: 'inherit',
  maxPeers: '',
  maxQueue: '',
  holdSecs: '',
});

const draftFrom = (r: ManagedRoomEntry): Draft => ({
  id: r.id,
  kind: r.kind,
  video: r.video == null ? 'inherit' : r.video ? 'on' : 'off',
  maxPeers: r.floor?.max_peers?.toString() ?? '',
  maxQueue: r.floor?.max_queue?.toString() ?? '',
  holdSecs: r.floor?.hold_timeout_secs?.toString() ?? '',
});

const floorSummary = (r: ManagedRoomEntry): string => {
  const f = r.floor;
  if (!f) return 'global';
  const parts = [
    f.max_peers !== undefined ? `peers ${f.max_peers}` : '',
    f.max_queue !== undefined ? `queue ${f.max_queue}` : '',
    f.hold_timeout_secs !== undefined ? `hold ${f.hold_timeout_secs}s` : '',
  ].filter(Boolean);
  return parts.length ? parts.join(' · ') : 'global';
};

export default function Rooms() {
  const { isAdmin } = useAuth();
  const [rooms, setRooms] = useState<ManagedRoomEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ mode: 'create' | 'edit'; draft: Draft } | null>(null);
  const [deleting, setDeleting] = useState<ManagedRoomEntry | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(() => {
    listManagedRooms()
      .then((r) => {
        setRooms(r.rooms);
        setError(null);
      })
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const save = (e?: FormEvent) => {
    e?.preventDefault();
    if (!editing) return;
    const d = editing.draft;
    if (editing.mode === 'create' && !d.id.trim()) {
      setError('room id is required');
      return;
    }
    const floor =
      d.maxPeers || d.maxQueue || d.holdSecs
        ? {
            ...(d.maxPeers ? { max_peers: Number(d.maxPeers) } : {}),
            ...(d.maxQueue ? { max_queue: Number(d.maxQueue) } : {}),
            ...(d.holdSecs ? { hold_timeout_secs: Number(d.holdSecs) } : {}),
          }
        : null;
    const video = d.video === 'inherit' ? null : d.video === 'on';
    setBusy(true);
    const p =
      editing.mode === 'create'
        ? createManagedRoom({ id: d.id.trim(), kind: d.kind, video, floor })
        : updateManagedRoom(d.id, { kind: d.kind, video, floor });
    p.then(() => {
      setEditing(null);
      setBusy(false);
      refresh();
    }).catch((e: unknown) => {
      setBusy(false);
      setError(e instanceof Error ? e.message : String(e));
    });
  };

  const remove = () => {
    if (!deleting) return;
    setBusy(true);
    deleteManagedRoom(deleting.id)
      .then(() => {
        setDeleting(null);
        setBusy(false);
        refresh();
      })
      .catch((e: unknown) => {
        setBusy(false);
        setError(e instanceof Error ? e.message : String(e));
      });
  };

  return (
    <section>
      <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--ms-space-3)' }}>
        <h1 style={{ margin: 0 }}>Rooms</h1>
        <Button
          variant="primary"
          disabled={!isAdmin}
          disabledReason="admin only — dispatchers have read access"
          onClick={() => setEditing({ mode: 'create', draft: emptyDraft() })}
        >
          New room
        </Button>
      </div>
      <p role="note" style={{ color: 'var(--ms-color-text-muted)', fontSize: 'var(--ms-font-size-sm)' }}>
        Managed rooms (rooms.yaml). Floor limits: blank fields inherit the global MEDIASERVO_FLOOR_*
        resolution. Deleting removes the config record — live sessions continue until peers leave.
      </p>
      {error && <p role="alert">{error}</p>}
      {rooms === null && !error && <p role="status">Loading rooms…</p>}
      {rooms !== null && rooms.length === 0 && (
        <p role="status">No managed rooms yet — create one to designate a talk room.</p>
      )}
      {rooms !== null && rooms.length > 0 && (
        <Table>
          <THead>
            <TR>
              <TH>id</TH>
              <TH>kind</TH>
              <TH>video</TH>
              <TH>floor limits</TH>
              <TH>actions</TH>
            </TR>
          </THead>
          <TBody>
            {rooms.map((r) => (
              <TR key={r.id}>
                <TD>{r.id}</TD>
                <TD>{r.kind}</TD>
                <TD>{r.video == null ? 'inherit' : r.video ? 'on' : 'off'}</TD>
                <TD>{floorSummary(r)}</TD>
                <TD>
                  <span style={{ display: 'flex', gap: 'var(--ms-space-2)' }}>
                    <Button
                      size="sm"
                      disabled={!isAdmin}
                      disabledReason="admin only"
                      onClick={() => setEditing({ mode: 'edit', draft: draftFrom(r) })}
                    >
                      Edit
                    </Button>
                    <Button
                      size="sm"
                      variant="danger"
                      disabled={!isAdmin}
                      disabledReason="admin only"
                      onClick={() => setDeleting(r)}
                    >
                      Delete
                    </Button>
                  </span>
                </TD>
              </TR>
            ))}
          </TBody>
        </Table>
      )}
      {editing && (
        <Modal
          open
          title={editing.mode === 'create' ? 'New room' : `Edit ${editing.draft.id}`}
          onClose={() => setEditing(null)}
          actions={
            <>
              <Button onClick={() => setEditing(null)}>Cancel</Button>
              <Button variant="primary" disabled={busy} onClick={() => save()}>
                Save
              </Button>
            </>
          }
        >
          <form onSubmit={save} style={{ display: 'grid', gap: 'var(--ms-space-3)' }}>
            <Input
              label="id"
              value={editing.draft.id}
              disabled={editing.mode === 'edit'}
              onChange={(e) => setEditing({ ...editing, draft: { ...editing.draft, id: e.target.value } })}
            />
            <label style={{ display: 'grid', gap: 'var(--ms-space-1)', fontSize: 'var(--ms-font-size-sm)' }}>
              kind
              <Combobox
                options={KIND_OPTIONS}
                value={editing.draft.kind}
                onChange={(v) => setEditing({ ...editing, draft: { ...editing.draft, kind: v } })}
                aria-label="room kind"
              />
            </label>
            <label style={{ display: 'grid', gap: 'var(--ms-space-1)', fontSize: 'var(--ms-font-size-sm)' }}>
              video
              <Combobox
                options={VIDEO_OPTIONS}
                value={editing.draft.video}
                onChange={(v) => setEditing({ ...editing, draft: { ...editing.draft, video: v as Draft['video'] } })}
                aria-label="video flag"
              />
            </label>
            <Input
              label="max_peers"
              hint="blank = global default"
              inputMode="numeric"
              value={editing.draft.maxPeers}
              onChange={(e) => setEditing({ ...editing, draft: { ...editing.draft, maxPeers: e.target.value } })}
            />
            <Input
              label="max_queue"
              hint="blank = global default"
              inputMode="numeric"
              value={editing.draft.maxQueue}
              onChange={(e) => setEditing({ ...editing, draft: { ...editing.draft, maxQueue: e.target.value } })}
            />
            <Input
              label="hold_timeout_secs"
              hint="blank = global default"
              inputMode="numeric"
              value={editing.draft.holdSecs}
              onChange={(e) => setEditing({ ...editing, draft: { ...editing.draft, holdSecs: e.target.value } })}
            />
          </form>
        </Modal>
      )}
      {deleting && (
        <Confirm
          open
          title={`Delete ${deleting.id}?`}
          message="The config record is removed. Live sessions in this room continue until peers leave."
          confirmLabel="Delete"
          tone="danger"
          onConfirm={remove}
          onCancel={() => setDeleting(null)}
        />
      )}
    </section>
  );
}
