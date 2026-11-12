import { useCallback, useEffect, useState } from "react";
import { api, type SessionInfo } from "./api";
import { Dashboard } from "./components/Dashboard";
import { Login } from "./components/Login";

export default function App() {
  const [session, setSession] = useState<SessionInfo | null>(null);
  const [loading, setLoading] = useState(true);

  // Auto-login: the backend restores the stored (DPAPI-encrypted) session.
  const loadSession = useCallback(async () => {
    try {
      setSession(await api.getSession());
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    loadSession();
  }, [loadSession]);

  if (loading) {
    return <div className="flex h-full items-center justify-center text-sm text-slate-500">Loading…</div>;
  }
  if (!session) {
    return <Login onLoggedIn={loadSession} />;
  }
  return <Dashboard session={session} onLoggedOut={() => setSession(null)} />;
}
