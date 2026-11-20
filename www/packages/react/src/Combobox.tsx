import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';

export interface ComboboxOption {
  value: string;
  label: string;
}

export interface ComboboxProps {
  options: ComboboxOption[];
  value: string | null;
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  id?: string;
  className?: string;
  'aria-label'?: string;
}

/**
 * Custom combobox (design rule: native <select> is banned).
 * role=combobox + listbox + aria-activedescendant; keyboard:
 * Arrows/Home/End move, Enter commits, Escape reverts, Tab closes.
 */
export function Combobox({
  options,
  value,
  onChange,
  placeholder,
  disabled,
  id,
  className = '',
  ...aria
}: ComboboxProps) {
  const autoId = useId();
  const cbId = id ?? autoId;
  const listId = `${cbId}-listbox`;
  const rootRef = useRef<HTMLDivElement>(null);

  const selected = options.find((o) => o.value === value) ?? null;
  const [open, setOpen] = useState(false);
  /** null = display the selected label; a string = the filter query being typed. */
  const [query, setQuery] = useState<string | null>(null);
  const [active, setActive] = useState(0);

  const filtered =
    query === null ? options : options.filter((o) => o.label.toLowerCase().includes(query.toLowerCase()));
  const activeIdx = filtered.length ? Math.min(active, filtered.length - 1) : 0;
  const activeId = filtered.length ? `${cbId}-opt-${activeIdx}` : undefined;

  // close on outside pointer
  useEffect(() => {
    if (!open) return;
    function onDocPointerDown(e: PointerEvent) {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpen(false);
        setQuery(null);
      }
    }
    document.addEventListener('pointerdown', onDocPointerDown);
    return () => document.removeEventListener('pointerdown', onDocPointerDown);
  }, [open]);

  function openList() {
    const selIdx = selected ? options.findIndex((o) => o.value === selected.value) : 0;
    setActive(selIdx >= 0 ? selIdx : 0);
    setQuery(null);
    setOpen(true);
  }

  function commit(idx: number) {
    const o = filtered[idx];
    if (!o) return;
    onChange(o.value);
    setQuery(null);
    setOpen(false);
  }

  function onKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        if (!open) openList();
        else setActive((a) => Math.min(a + 1, filtered.length - 1));
        break;
      case 'ArrowUp':
        e.preventDefault();
        if (!open) openList();
        else setActive((a) => Math.max(a - 1, 0));
        break;
      case 'Home':
        if (open) {
          e.preventDefault();
          setActive(0);
        }
        break;
      case 'End':
        if (open) {
          e.preventDefault();
          setActive(Math.max(filtered.length - 1, 0));
        }
        break;
      case 'Enter':
        e.preventDefault();
        if (open && filtered[activeIdx]) commit(activeIdx);
        else if (!open) openList();
        break;
      case 'Escape':
        if (open) {
          e.preventDefault();
          setOpen(false);
          setQuery(null);
        }
        break;
      case 'Tab':
        setOpen(false);
        setQuery(null);
        break;
    }
  }

  return (
    <div ref={rootRef} className={['msr-cb', className].filter(Boolean).join(' ')}>
      <div className="msr-cb-box">
        <input
          {...aria}
          id={cbId}
          role="combobox"
          className="msr-input msr-cb-input"
          disabled={disabled}
          placeholder={placeholder}
          autoComplete="off"
          aria-controls={listId}
          aria-expanded={open}
          aria-haspopup="listbox"
          aria-autocomplete="list"
          aria-activedescendant={open ? activeId : undefined}
          value={query ?? selected?.label ?? ''}
          onChange={(e) => {
            setQuery(e.target.value);
            setActive(0);
            setOpen(true);
          }}
          onFocus={() => {
            if (!open) openList();
          }}
          onKeyDown={onKeyDown}
        />
        <button
          type="button"
          className="msr-cb-toggle"
          tabIndex={-1}
          aria-hidden="true"
          aria-expanded={open}
          disabled={disabled}
          onClick={() => (open ? (setOpen(false), setQuery(null)) : openList())}
        />
      </div>
      {open && (
        <ul id={listId} role="listbox" className="msr-cb-list" aria-labelledby={cbId}>
          {filtered.map((o, i) => (
            <li
              key={o.value}
              id={`${cbId}-opt-${i}`}
              role="option"
              aria-selected={o.value === value}
              className={['msr-cb-option', i === activeIdx && 'is-active'].filter(Boolean).join(' ')}
              onPointerDown={(e) => {
                e.preventDefault(); // keep input focus
                commit(i);
              }}
              onMouseEnter={() => setActive(i)}
            >
              {o.label}
            </li>
          ))}
          {!filtered.length && (
            <li className="msr-cb-empty" role="presentation">
              No matches
            </li>
          )}
        </ul>
      )}
    </div>
  );
}
