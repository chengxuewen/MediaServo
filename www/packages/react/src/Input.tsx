import { useId, type InputHTMLAttributes } from 'react';
import type { ControlSize } from './Button';

export interface InputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'className' | 'size'> {
  /** Wraps into .msr-field with label/hint/error when any is present; bare input otherwise. */
  label?: string;
  hint?: string;
  error?: string;
  mono?: boolean;
  size?: ControlSize;
  className?: string;
}

export function Input({ label, hint, error, mono, size = 'md', id, className = '', ...rest }: InputProps) {
  const autoId = useId();
  const inputId = id ?? rest.name ?? autoId;
  const describedBy =
    [hint ? `${inputId}-hint` : '', error ? `${inputId}-err` : ''].filter(Boolean).join(' ') || undefined;
  const cls = [
    'msr-input',
    mono && 'msr-input--mono',
    size === 'sm' && 'msr-input--sm',
    error && 'msr-input--invalid',
    className,
  ]
    .filter(Boolean)
    .join(' ');

  const input = (
    <input
      id={inputId}
      {...rest}
      className={cls}
      aria-invalid={error ? true : undefined}
      aria-describedby={describedBy}
    />
  );

  if (!label && !hint && !error) return input;
  return (
    <div className="msr-field">
      {label && (
        <label className="msr-label" htmlFor={inputId}>
          {label}
        </label>
      )}
      {input}
      {hint && (
        <div className="msr-hint" id={`${inputId}-hint`}>
          {hint}
        </div>
      )}
      {error && (
        <div className="msr-error" id={`${inputId}-err`} role="alert">
          {error}
        </div>
      )}
    </div>
  );
}
