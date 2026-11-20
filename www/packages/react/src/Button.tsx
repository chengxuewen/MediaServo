import type { ButtonHTMLAttributes, ReactNode } from 'react';

export type ButtonVariant = 'primary' | 'secondary' | 'danger' | 'ghost';
export type ControlSize = 'sm' | 'md';

export interface ButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'className'> {
  variant?: ButtonVariant;
  size?: ControlSize;
  /** Guarded state (design rule: never native disabled). */
  disabled?: boolean;
  /** Shown as data-reason tooltip when disabled; guards must carry a reason. */
  disabledReason?: string;
  className?: string;
  children?: ReactNode;
}

export function Button({
  variant = 'secondary',
  size = 'md',
  disabled = false,
  disabledReason,
  className = '',
  type = 'button',
  children,
  onClick,
  ...rest
}: ButtonProps) {
  const cls = ['msr-btn', `msr-btn--${variant}`, size === 'sm' ? 'msr-btn--sm' : '', className]
    .filter(Boolean)
    .join(' ');
  return (
    <button
      {...rest}
      type={type}
      className={cls}
      aria-disabled={disabled || undefined}
      data-reason={disabled ? (disabledReason ?? '') : undefined}
      tabIndex={disabled ? 0 : undefined}
      onClick={disabled ? undefined : onClick}
    >
      {children}
    </button>
  );
}
