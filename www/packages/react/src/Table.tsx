import type {
  HTMLAttributes,
  TableHTMLAttributes,
  TdHTMLAttributes,
  ThHTMLAttributes,
} from 'react';

export interface TableProps extends TableHTMLAttributes<HTMLTableElement> {
  dense?: boolean;
}

/** Styled table primitives — consumers compose rows/cells (keeps typing native). */
export function Table({ dense, className = '', ...rest }: TableProps) {
  return (
    <table {...rest} className={['msr-table', dense && 'msr-table--dense', className].filter(Boolean).join(' ')} />
  );
}

export function THead(props: HTMLAttributes<HTMLTableSectionElement>) {
  return <thead {...props} />;
}

export function TBody(props: HTMLAttributes<HTMLTableSectionElement>) {
  return <tbody {...props} />;
}

export function TR(props: HTMLAttributes<HTMLTableRowElement>) {
  return <tr {...props} />;
}

export function TH(props: ThHTMLAttributes<HTMLTableCellElement>) {
  return <th {...props} />;
}

export function TD(props: TdHTMLAttributes<HTMLTableCellElement>) {
  return <td {...props} />;
}
