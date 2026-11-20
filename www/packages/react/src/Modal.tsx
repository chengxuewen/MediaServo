import { useEffect, useId, useRef, type ReactNode } from 'react';
import { createPortal } from 'react-dom';

const FOCUSABLE =
  'a[href], button:not([tabindex="-1"]), input:not([tabindex="-1"]), select:not([tabindex="-1"]), textarea:not([tabindex="-1"]), [tabindex]:not([tabindex="-1"])';

interface StackEntry {
  el: HTMLElement;
  prev: HTMLElement | null;
}

/** Dialog stack (PIT-269): only the TOP dialog arbitrates Esc; focus restore is LIFO. */
const stack: StackEntry[] = [];

export interface ModalProps {
  open: boolean;
  title?: ReactNode;
  onClose: () => void;
  children?: ReactNode;
  /** Action row (buttons) rendered in the footer. */
  actions?: ReactNode;
  /** Extra width class, e.g. 'msr-modal--wide'. */
  className?: string;
}

export function Modal({ open, title, onClose, children, actions, className = '' }: ModalProps) {
  const panelRef = useRef<HTMLDivElement>(null);
  const dlgId = useId();
  // keep the arbitration effect stable regardless of the consumer's onClose identity
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    if (!open) return;
    const el = panelRef.current;
    if (!el) return;

    const entry: StackEntry = {
      el,
      prev: document.activeElement instanceof HTMLElement ? document.activeElement : null,
    };
    stack.push(entry);

    // initial focus: first focusable, else the panel itself
    const first = el.querySelector<HTMLElement>(FOCUSABLE);
    (first ?? el).focus();

    function onKey(e: KeyboardEvent) {
      if (stack[stack.length - 1] !== entry) return; // only the top dialog arbitrates
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        onCloseRef.current();
        return;
      }
      if (e.key === 'Tab') {
        const items = Array.from(entry.el.querySelectorAll<HTMLElement>(FOCUSABLE));
        if (!items.length) return;
        const firstItem = items[0];
        const lastItem = items[items.length - 1];
        const current = document.activeElement;
        if (e.shiftKey && current === firstItem) {
          e.preventDefault();
          lastItem.focus();
        } else if (!e.shiftKey && current === lastItem) {
          e.preventDefault();
          firstItem.focus();
        }
      }
    }

    document.addEventListener('keydown', onKey, true);
    return () => {
      document.removeEventListener('keydown', onKey, true);
      const i = stack.indexOf(entry);
      if (i >= 0) stack.splice(i, 1);
      // LIFO focus restore; skip if the origin left the DOM (detached fallback)
      if (entry.prev && document.contains(entry.prev)) entry.prev.focus();
    };
  }, [open]);

  if (!open) return null;

  return createPortal(
    <div
      className="msr-scrim"
      onPointerDown={(e) => {
        if (e.target === e.currentTarget) onCloseRef.current();
      }}
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={title ? `${dlgId}-title` : undefined}
        tabIndex={-1}
        className={['msr-modal', className].filter(Boolean).join(' ')}
        onPointerDown={(e) => e.stopPropagation()}
      >
        {title && (
          <div className="msr-modal__head">
            <h2 id={`${dlgId}-title`} className="msr-modal__title">
              {title}
            </h2>
          </div>
        )}
        <div className="msr-modal__body">{children}</div>
        {actions && <div className="msr-modal__actions">{actions}</div>}
      </div>
    </div>,
    document.body,
  );
}
