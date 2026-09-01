import { deleteRoom } from '../api/client';
import type { Consumer } from '../api/client';
import './StreamDetail.css';
import { Video, X, Eye } from 'lucide-react';

interface Props {
  deviceId: string;
  streamId: string;
  consumers: Consumer[];
  onClose: () => void;
  onKick?: (peerId: string) => void;
  canManage?: boolean;
}

export default function StreamDetail({ deviceId, streamId, consumers, onClose, onKick, canManage = true }: Props) {
  return (
    <div className="stream-detail-overlay" onClick={onClose}>
      <div className="stream-detail" onClick={(e) => e.stopPropagation()}>
        <div className="detail-header">
          <h3><Video size={15} /> {streamId}</h3>
          <button className="close-btn" onClick={onClose}><X size={16} /></button>
        </div>
        <div className="detail-body">
          <div className="detail-row">
            <span className="detail-label">Device</span>
            <span>{deviceId}</span>
          </div>
          <div className="detail-row">
            <span className="detail-label">Consumers</span>
            <span>{consumers.length}</span>
          </div>
          {consumers.length > 0 && (
            <ul className="consumer-list-detail">
              {consumers.map((c) => (
                <li key={c.peer_id} className="consumer-item">
                  <span><Eye size={13} /> {c.peer_id}</span>
                  <span className="consumer-since">{(c.connected_since ?? '').slice(0, 19)}</span>
                  {onKick && (
                    <button className="btn-sm" onClick={() => onKick(c.peer_id)}>Kick</button>
                  )}
                </li>
              ))}
            </ul>
          )}
          {canManage && (
            <button
              className="btn-danger"
              onClick={async () => { await deleteRoom(`${deviceId}_${streamId}`); onClose(); }}
            >
              Close Stream
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
