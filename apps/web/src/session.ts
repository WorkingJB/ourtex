// Session-token persistence for the web client.
//
// localStorage is a pragmatic first pass: same-origin only, survives a
// reload, lets the bearer attach from a single source. It is vulnerable
// to XSS; moving to an httpOnly cookie issued by the server is a
// follow-up once 2b.5 hardens the auth surface end-to-end.
const KEY = "ourtex.session.v1";

export type StoredSession = {
  token: string;
  accountId: string;
  email: string;
  displayName: string;
  expiresAt: string;
};

export function loadSession(): StoredSession | null {
  const raw = localStorage.getItem(KEY);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as StoredSession;
    if (new Date(parsed.expiresAt).getTime() <= Date.now()) {
      localStorage.removeItem(KEY);
      return null;
    }
    return parsed;
  } catch {
    localStorage.removeItem(KEY);
    return null;
  }
}

export function saveSession(session: StoredSession): void {
  localStorage.setItem(KEY, JSON.stringify(session));
}

export function clearSession(): void {
  localStorage.removeItem(KEY);
}
