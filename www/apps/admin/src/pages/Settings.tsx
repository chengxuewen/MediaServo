import { useState } from 'react';
import { setToken, clearToken, hasToken, login, getRole, getUsername, getPsk, rotatePsk } from '../api/client';
import Modal from '../components/Modal';
import './Settings.css';
import { AlertTriangle, Check } from 'lucide-react';

export default function Settings() {
  const [username, setUsernameInput] = useState('');
  const [password, setPassword] = useState('');
  const [token, setTokenInput] = useState('');
  const [saved, setSaved] = useState(hasToken());
  const [showSaved, setShowSaved] = useState(false);
  const [loginError, setLoginError] = useState<string | null>(null);
  const [loggingIn, setLoggingIn] = useState(false);
  const currentUser = getUsername();
  const currentRole = getRole();
  const isAdmin = currentRole === 'admin';

  // ── PSK 管理（psk-admin-management）──────────────────────────────────
  const [pskSecret, setPskSecret] = useState<{ psk: string; hint: string } | null>(null);
  const [pskCopied, setPskCopied] = useState(false);
  const [pskError, setPskError] = useState<string | null>(null);
  const [pskNotice, setPskNotice] = useState<string | null>(null);
  const [pskBusy, setPskBusy] = useState(false);

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
      setPskCopied(false);
    } catch (e) {
      setPskError(e instanceof Error ? e.message : 'View failed');
    } finally {
      setPskBusy(false);
    }
  };

  const handleRotatePsk = async () => {
    if (!window.confirm('Rotate the server PSK? All hosts must sync the new key immediately.')) return;
    setPskBusy(true);
    setPskError(null);
    setPskNotice(null);
    try {
      const resp = await rotatePsk();
      setPskSecret(resp);
      setPskCopied(false);
      setPskNotice('PSK rotated — sync every host with the new key');
    } catch (e) {
      setPskError(e instanceof Error ? e.message : 'Rotate failed');
    } finally {
      setPskBusy(false);
    }
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
  };

  return (
    <div className="settings">
      <h2>Settings</h2>

      <section className="setting-group">
        <h3>Account Login</h3>
        <p className="setting-desc">Sign in with a cockpit account (G3 accounts.yaml — viewer/operator/admin/dispatcher). The token is stored locally; role-aware views apply.</p>
        {currentUser && (
          <p className="token-status saved"><Check size={14} /> Signed in as {currentUser}{currentRole ? ` [${currentRole}]` : ''}</p>
        )}
        <div className="token-row">
          <input
            type="text"
            className="login-input"
            placeholder="username"
            value={username}
            onChange={(e) => setUsernameInput(e.target.value)}
          />
          <input
            type="password"
            className="login-input"
            placeholder="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
          <button className="btn" onClick={handleLogin} disabled={loggingIn}>
            {loggingIn ? 'Signing in...' : 'Login'}
          </button>
        </div>
        {loginError && <p className="token-status error">{loginError}</p>}
      </section>

      <section className="setting-group">
        <h3>Admin Token</h3>
        <p className="setting-desc">Or paste the admin JWT token from the server startup output, or generate one with <code>--create-admin-token</code>.</p>
        <div className="token-row">
          <input
            type="text"
            className="token-input"
            placeholder="eyJhbGciOiJIUzI1NiIs..."
            value={token}
            onChange={(e) => setTokenInput(e.target.value)}
          />
          <button className="btn" onClick={handleSave}>Save Token</button>
          {saved && <button className="btn btn-outline" onClick={handleClear}>Clear</button>}
        </div>
        {showSaved && <p className="token-status saved"><Check size={14} /> Token saved</p>}
      </section>

      {isAdmin && (
        <section className="setting-group">
          <h3>PSK Management</h3>
          <p className="setting-desc">Server shared key (psk-admin-management). One-time view; rotation invalidates all hosts immediately — sync every host after rotating.</p>
          <div className="token-row">
            <span className="psk-hint">{pskSecret ? `current: ${pskSecret.hint}` : ''}</span>
            <button className="btn" onClick={handleViewPsk} disabled={pskBusy}>View</button>
            <button className="btn btn-outline" onClick={handleRotatePsk} disabled={pskBusy}>
              {pskBusy ? 'Working...' : 'Rotate'}
            </button>
          </div>
          {pskError && <p className="token-status error">{pskError}</p>}
          {pskNotice && <p className="token-status saved">{pskNotice}</p>}
        </section>
      )}

      <section className="setting-group">
        <h3>About</h3>
        <p>MediaServo Admin Dashboard v0.1.0</p>
        <p className="setting-desc">Remote control scenario — monitor device streams, consumers, audio conference rooms, and vehicle status.</p>
      </section>

      {pskSecret && (
        <Modal title="PSK (shown once)" onClose={() => setPskSecret(null)}>
          <p className="token-status saved"><AlertTriangle size={14} /> This key is shown only once — copy it now, then sync all hosts.</p>
          <textarea
            className="secret-box"
            readOnly
            value={pskSecret.psk}
            rows={3}
            onFocus={(e) => e.target.select()}
          />
          {pskError && <p className="token-status error">{pskError}</p>}
          <div className="token-row">
            <button className="btn" onClick={copyPsk}>{pskCopied ? <><Check size={12} /> Copied</> : 'Copy'}</button>
            <button className="btn btn-outline" onClick={() => setPskSecret(null)}>Done</button>
          </div>
        </Modal>
      )}
    </div>
  );
}
