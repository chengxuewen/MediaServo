import type { ReactNode } from 'react';

export interface GuardProps {
  /** Why the control is unavailable — surfaced on hover/focus (design rule: guards carry reasons). */
  reason: string;
  children: ReactNode;
  className?: string;
}

/**
 * Disabled-with-reason host (design rule: never native `disabled`).
 * The child stays visible and focusable; pointer/Enter/Space activation is
 * blocked at the wrapper so the reason tooltip stays discoverable.
 */
export function Guard({ reason, children, className = '' }: GuardProps) {
  return (
    <span
      className={['msr-guard', className].filter(Boolean).join(' ')}
      aria-disabled="true"
      data-reason={reason}
      onPointerDown={(e) => {
        e.preventDefault();
        e.stopPropagation();
      }}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          e.stopPropagation();
        }
      }}
    >
      {children}
    </span>
  );
}
