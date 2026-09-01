import type { ReactNode } from 'react';
import './Modal.css';
import { X } from 'lucide-react';

interface Props {
  title: string;
  children: ReactNode;
  onClose: () => void;
}

/** 居中弹窗（Devices/Accounts 表单共用; 复用 stream-detail 的 overlay 交互语义）。 */
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