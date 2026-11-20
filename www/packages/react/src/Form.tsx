import type { FormEvent, FormHTMLAttributes, ReactNode } from 'react';

export interface FormProps extends Omit<FormHTMLAttributes<HTMLFormElement>, 'className' | 'onSubmit'> {
  /** Called after the default submission is prevented. */
  onSubmit?: (e: FormEvent<HTMLFormElement>) => void;
  className?: string;
}

export function Form({ onSubmit, className = '', ...rest }: FormProps) {
  return (
    <form
      {...rest}
      noValidate
      className={['msr-form', className].filter(Boolean).join(' ')}
      onSubmit={(e) => {
        e.preventDefault();
        onSubmit?.(e);
      }}
    />
  );
}

export interface FieldProps {
  label?: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
  htmlFor?: string;
  children: ReactNode;
  className?: string;
}

/** Layout wrapper: label + control + hint/error. Pairs with Input or custom controls. */
export function Field({ label, hint, error, htmlFor, children, className = '' }: FieldProps) {
  return (
    <div className={['msr-field', className].filter(Boolean).join(' ')}>
      {label && (
        <label className="msr-label" htmlFor={htmlFor}>
          {label}
        </label>
      )}
      {children}
      {hint && <div className="msr-hint">{hint}</div>}
      {error && (
        <div className="msr-error" role="alert">
          {error}
        </div>
      )}
    </div>
  );
}
