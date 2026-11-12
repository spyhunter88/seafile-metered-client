import { useState, type FormEvent } from "react";
import { api, errorMessage } from "../api";

export function Login({ onLoggedIn }: { onLoggedIn: () => void }) {
  const [serverUrl, setServerUrl] = useState("https://");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.login(serverUrl, email, password);
      setPassword("");
      onLoggedIn();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  const input =
    "w-full rounded-md border border-slate-300 bg-white px-3 py-2 text-sm outline-none focus:border-sky-500 focus:ring-2 focus:ring-sky-100";

  return (
    <div className="flex h-full items-center justify-center p-6">
      <form onSubmit={submit} className="w-full max-w-sm space-y-4 rounded-xl bg-white p-6 shadow-sm ring-1 ring-slate-200">
        <div>
          <h1 className="text-lg font-semibold">Seafile Metered Client</h1>
          <p className="text-xs text-slate-500">Sign in to your Seafile server</p>
        </div>

        <label className="block space-y-1">
          <span className="text-xs font-medium text-slate-600">Server URL</span>
          <input className={input} value={serverUrl} onChange={(e) => setServerUrl(e.target.value)} placeholder="https://cloud.example.com" required />
        </label>
        <label className="block space-y-1">
          <span className="text-xs font-medium text-slate-600">Email</span>
          <input className={input} type="text" value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="username" required />
        </label>
        <label className="block space-y-1">
          <span className="text-xs font-medium text-slate-600">Password</span>
          <input className={input} type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" required />
        </label>

        {error && <div className="rounded-md bg-rose-50 px-3 py-2 text-xs text-rose-700">{error}</div>}

        <button
          type="submit"
          disabled={busy}
          className="w-full rounded-md bg-sky-600 px-3 py-2 text-sm font-medium text-white hover:bg-sky-700 disabled:opacity-60"
        >
          {busy ? "Signing in…" : "Sign in"}
        </button>
      </form>
    </div>
  );
}
