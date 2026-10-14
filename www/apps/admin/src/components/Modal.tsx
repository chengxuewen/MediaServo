import type { ReactNode } from 'react';
import './Modal.css';
import { X } from 'lucide-react';

interface Props {
  title: string;
  children: ReactNode;
  onClose: () => void;
}

/** Centered modal (shared by the Devices/Accounts forms; reuses the stream-detail overlay interaction semantics). */
export default function Modal({ title, children, onClose }: Props) {
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h3>{title}</h3>
          <button className="close-btn" onClick={onClose}><X size={16} /></button>
        </div>
        <div className="modal-body">{children}</div>
      </div>
    </div>
  );
}