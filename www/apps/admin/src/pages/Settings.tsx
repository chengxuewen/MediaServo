// W4 (admin-prototype-restyle, B2 approved real face + current placement per
// adjudication 2): PSK card + admin bootstrap token card stay HERE (Keys page
// cross-links); card language per prototype. Rotate gains a confirm modal with
// optional custom value (server validates 8-128); one-time secrets are
// close-protected; Clear confirms. The AccessBase provider layer is deferred.
import { useState } from 'react';
import { Button, Confirm, Input, Modal } from '@mediaservo/react';
import { setToken, clearToken, hasToken, login, getRole, getUsername, getPsk, rotatePsk } from '../api/client';
import { AlertTriangle, Check } from 'lucide-react';

const cardStyle = {
  background: 'var(--ms-color-surface)',
  border: '1px solid var(--ms-color-border)',
  borderRadius: 'var(--ms-radius-md)',
  padding: 'var(--ms-space-4)',
  marginBottom: 'var(--ms-space-3)',
} as const;

const cardTitle = {
  fontSize: 'var(--ms-font-size-sm)',
  fontWeight: 'var(--ms-font-weight-semibold)',
  margin: '0 0 var(--ms-space-2)',
} as const;

const descStyle = {
  fontSize: 'var(--ms-font-size-xs)',
  color: 'var(--ms-color-text-tertiary)',
  margin: '0 0 var(--ms-space-2)',
} as const;

const rowStyle = { display: 'flex', gap: 'var(--ms-space-2)', alignItems: 'center', flexWrap: 'wrap' } as const;

const monoStyle = { fontFamily: 'var(--ms-font-family-mono)' } as const;

const chipStyle = {
  display: 'inline-block',
  padding: '0 var(--ms-space-2)',
  border: '1px solid var(--ms-color-border)',
  borderRadius: 'var(--ms-radius-full)',
  fontSize: 'var(--ms-font-size-xs)',
  color: 'var(--ms-color-text-secondary)',
} as const;

export default function Settings() {
  const [username, setUsernameInput] = useState('');
  const [password, setPassword] = useState('');
  const [token, setTokenInput] = useState('');
  const [saved, setSaved] = useState(hasToken());
  const [showSaved, setShowSaved] = useState(false);
  const [loginError, setLoginError] = useState<string | null>(null);
  // V20: theme lives here (single source of truth) — the monitor wall applies it.
  const [theme, setThemeState] = useState<string>(() => { try { return localStorage.getItem('msrv_theme') || 'light'; } catch { return 'light'; } });
  const applyTheme = (t: string) => {
    setThemeState(t);
    try { localStorage.setItem('msrv_theme', t); document.documentElement.dataset.theme = t; } catch { /* silent */ }
  };
  const [loggingIn, setLoggingIn] = useState(false);
  const [clearOpen, setClearOpen] = useState(false);
  const currentUser = getUsername();
  const currentRole = getRole();
  const isAdmin = currentRole === 'admin';

  // ── PSK management (psk-admin-management) ─────────────────────────────
  const [pskSecret, setPskSecret] = useState<{ psk: string; hint: string } | null>(null);
  const [pskCopied, setPskCopied] = useState(false);
  const [pskError, setPskError] = useState<string | null>(null);
  const [pskNotice, setPskNotice] = useState<string | null>(null);
  const [pskBusy, setPskBusy] = useState(false);
  const [pskHint, setPskHint] = useState<string | null>(null);
  const [rotateOpen, setRotateOpen] = useState(false);
  const [customPsk, setCustomPsk] = useState('');
  const [discardOpen, setDiscardOpen] = useState(false);

  const copyPsk = async () => {
    if (!pskSecret) return;
    try {
      await navigator.clipboard.writeText(pskSecret.psk);
      setPskCopied(true);
      setPskError(null);
    } catch {
      setPskError('Clipboard unavailable — select the secret manually');
    }
  };

  const handleViewPsk = async () => {
    setPskBusy(true);
    setPskError(null);
    setPskNotice(null);
    try {
      const resp = await getPsk();
      setPskSecret(resp);
      setPskHint(resp.hint);
      setPskCopied(false);
    } catch (e) {
      setPskError(e instanceof Error ? e.message : 'View failed');
    } finally {
      setPskBusy(false);
    }
  };

  const handleRotatePsk = async () => {
    setPskBusy(true);
    setPskError(null);
    setPskNotice(null);
    try {
      const resp = await rotatePsk(customPsk.trim() || undefined);
      setRotateOpen(false);
      setCustomPsk('');
      setPskSecret(resp);
      setPskHint(resp.hint);
      setPskCopied(false);
      setPskNotice('PSK rotated — sync every host with the new key');
    } catch (e) {
      setPskError(e instanceof Error ? e.message : 'Rotate failed');
      setRotateOpen(false);
    } finally {
      setPskBusy(false);
    }
  };

  const requestPskClose = () => {
    if (pskCopied) setPskSecret(null);
    else setDiscardOpen(true);
  };

  const handleLogin = async () => {
    if (!username.trim() || !password) return;
    setLoggingIn(true);
    setLoginError(null);
    try {
      const resp = await login(username.trim(), password);
      setToken(resp.token);
      setUsernameInput('');
      setPassword('');
      setSaved(true);
      setShowSaved(true);
    } catch (e) {
      setLoginError(e instanceof Error ? e.message : 'Login failed');
    } finally {
      setLoggingIn(false);
    }
  };

  const handleSave = () => {
    if (token.trim()) {
      setToken(token.trim());
      setTokenInput('');
      setSaved(true);
      setShowSaved(true);
    }
  };

  const handleClear = () => {
    clearToken();
    setSaved(false);
    setShowSaved(false);
    setClearOpen(false);
  };

  return (
    <section>
      <h1 style={{ margin: '0 0 var(--ms-space-3)' }}>Settings</h1>

      <section style={cardStyle} aria-label="Appearance">
        <h2 style={cardTitle}>Appearance</h2>
        <p style={descStyle}>Theme applies to the whole console, including the monitor wall.</p>
        <div style={rowStyle} role="group" aria-label="Theme">
          <Button variant={theme === 'light' ? 'primary' : undefined} aria-pressed={theme === 'light'} data-theme-choice="light" onClick={() => applyTheme('light')}>Light</Button>
          <Button variant={theme === 'dark' ? 'primary' : undefined} aria-pressed={theme === 'dark'} data-theme-choice="dark" onClick={() => applyTheme('dark')}>Dark</Button>
        </div>
      </section>

      <section style={cardStyle} aria-label="Account Login">
        <h2 style={cardTitle}>Account Login</h2>
        <p style={descStyle}>Sign in with a cockpit account (viewer / operator / dispatcher / admin). The token is stored locally; role-aware views apply.</p>
        {currentUser && (
          <p role="status" style={{ color: 'var(--ms-color-success)', fontSize: 'var(--ms-font-size-sm)', margin: '0 0 var(--ms-space-2)' }}>
            <Check size={14} /> Signed in as {currentUser}{currentRole ? ` [${currentRole}]` : ''}
          </p>
        )}
        <div style={rowStyle}>
          <Input placeholder="username" aria-label="Username" value={username} onChange={(e) => setUsernameInput(e.target.value)} style={{ flex: '0 0 160px' }} />
          <Input type="password" placeholder="password" aria-label="Password" value={password} onChange={(e) => setPassword(e.target.value)} style={{ flex: '0 0 160px' }} />
          <Button variant="primary" disabled={loggingIn} disabledReason="Signing in…" onClick={handleLogin}>
            {loggingIn ? 'Signing in...' : 'Login'}
          </Button>
        </div>
        {loginError && <p role="alert" style={{ color: 'var(--ms-color-danger)', fontSize: 'var(--ms-font-size-sm)' }}>{loginError}</p>}
      </section>

      <section style={cardStyle} aria-label="Admin bootstrap token">
        <h2 style={cardTitle}>Admin bootstrap token</h2>
        <p style={descStyle}>Paste the admin JWT from the server startup output (--create-admin-token). Save masks the value; Clear invalidates the local copy.</p>
        <div style={{ ...rowStyle, marginBottom: 'var(--ms-space-2)' }}>
          <span style={chipStyle}>{saved ? 'saved' : 'empty'}</span>
        </div>
        <div style={rowStyle}>
          <Input
            mono
            placeholder="eyJhbGciOiJIUzI1NiIs..."
            aria-label="Admin bootstrap token"
            value={token}
            onChange={(e) => setTokenInput(e.target.value)}
            style={{ flex: '1 1 220px' }}
          />
          <Button variant="primary" onClick={handleSave}>Save</Button>
          {saved && <Button onClick={() => setClearOpen(true)}>Clear</Button>}
        </div>
        {showSaved && (
          <p role="status" style={{ color: 'var(--ms-color-success)', fontSize: 'var(--ms-font-size-sm)' }}>
            <Check size={14} /> Token saved
          </p>
        )}
      </section>

      {isAdmin && (
        <section style={cardStyle} aria-label="Pre-shared key">
          <h2 style={cardTitle}>Pre-shared key (device/agent auth)</h2>
          <div style={{ ...rowStyle, marginBottom: 'var(--ms-space-2)' }}>
            <span style={{ fontSize: 'var(--ms-font-size-xs)', color: 'var(--ms-color-text-tertiary)' }}>current</span>
            <span style={{ ...monoStyle, fontSize: 'var(--ms-font-size-sm)' }}>{pskHint ?? '—'}</span>
            <span style={{ flex: 1 }} />
            <Button size="sm" disabled={pskBusy} disabledReason="Working…" onClick={handleViewPsk}>View</Button>
            <Button size="sm" disabled={pskBusy} disabledReason="Working…" onClick={() => { setCustomPsk(''); setRotateOpen(true); }}>Rotate</Button>
          </div>
          <p style={descStyle}>View reveals the full PSK and is audited each time (psk.view). Rotate shows the new value once — sync every host afterwards.</p>
          {pskError && <p role="alert" style={{ color: 'var(--ms-color-danger)', fontSize: 'var(--ms-font-size-sm)' }}>{pskError}</p>}
          {pskNotice && <p role="status" style={{ color: 'var(--ms-color-success)', fontSize: 'var(--ms-font-size-sm)' }}>{pskNotice}</p>}
        </section>
      )}

      <section style={cardStyle} aria-label="About">
        <h2 style={cardTitle}>About</h2>
        <p style={{ margin: 0, fontSize: 'var(--ms-font-size-sm)' }}>MediaServo Admin Dashboard v0.1.0</p>
        <p style={descStyle}>Remote control scenario — monitor device streams, consumers, audio conference rooms, and vehicle status.</p>
      </section>

      {clearOpen && (
        <Confirm
          open
          title="Clear the saved token?"
          message="The local copy is invalidated; you return to the login page on the next guarded visit."
          confirmLabel="Clear"
          tone="danger"
          onConfirm={handleClear}
          onCancel={() => setClearOpen(false)}
        />
      )}

      {rotateOpen && (
        <Modal
          open
          title="Rotate the server PSK?"
          onClose={() => setRotateOpen(false)}
          actions={
            <>
              <Button onClick={() => setRotateOpen(false)}>Cancel</Button>
              <Button variant="danger" disabled={pskBusy} disabledReason="Rotating…" onClick={handleRotatePsk}>Rotate</Button>
            </>
          }
        >
          <p role="alert" style={{ color: 'var(--ms-color-warning)', fontSize: 'var(--ms-font-size-sm)' }}>
            <AlertTriangle size={14} /> All hosts must sync the new key immediately. The new value is shown once.
          </p>
          <Input
            label="Custom value (optional)"
            type="password"
            value={customPsk}
            onChange={(e) => setCustomPsk(e.target.value)}
            placeholder="blank = server generates"
            hint="8-128 characters, no spaces or quotes; blank = random."
          />
        </Modal>
      )}

      {pskSecret && (
        <Modal
          open
          title="PSK (shown once)"
          onClose={requestPskClose}
          actions={
            <>
              <Button variant="primary" onClick={copyPsk}>{pskCopied ? <><Check size={12} /> Copied</> : 'Copy'}</Button>
              <Button onClick={requestPskClose}>Done</Button>
            </>
          }
        >
          <p role="alert" style={{ color: 'var(--ms-color-warning)', fontSize: 'var(--ms-font-size-sm)' }}>
            <AlertTriangle size={14} /> This key is shown only once — copy it now, then sync all hosts.
          </p>
          <textarea
            readOnly
            value={pskSecret.psk}
            rows={3}
            onFocus={(e) => e.target.select()}
            aria-label="PSK value"
            style={{
              width: '100%',
              ...monoStyle,
              fontSize: 'var(--ms-font-size-sm)',
              padding: 'var(--ms-space-2) var(--ms-space-3)',
              border: '1px solid var(--ms-color-border)',
              borderRadius: 'var(--ms-radius-sm)',
              resize: 'vertical',
              background: 'var(--ms-color-surface-subtle)',
              color: 'var(--ms-color-text)',
            }}
          />
          {pskError && <p role="alert">{pskError}</p>}
        </Modal>
      )}

      {discardOpen && (
        <Confirm
          open
          title="Discard the one-time PSK?"
          message="It was not copied. Closing loses it — view again (audited) or rotate to issue a new one."
          confirmLabel="Discard"
          tone="danger"
          onConfirm={() => { setDiscardOpen(false); setPskSecret(null); }}
          onCancel={() => setDiscardOpen(false)}
        />
      )}
    </section>
  );
}